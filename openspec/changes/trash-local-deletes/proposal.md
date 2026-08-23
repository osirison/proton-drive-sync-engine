## Why

When the engine mirrors a remote deletion onto disk it calls `fs::remove_file` /
`fs::remove_dir_all` (`src/daemon.rs:4155-4162`). The bytes are gone: no trash, no undo, no copy on
Proton to pull back. Every safety mechanism in the product is built on top of that fact — the
Deletions screen sorts local deletes into a `Permanent · this computer` column, puts a typed-`DELETE`
gate on each card, and the notifier fires an interrupting banner for them. The friction is real
because the risk is real.

The risk does not have to be real. On a desktop, deleting a file means moving it to the trash, and
the user can restore it from Files or Dolphin. Making that the engine's default removes the loss
this whole apparatus is defending against, and lets the warnings come down with it.

## What Changes

- A new per-pair config key `local_delete_mode` with values `trash` (the new default) and
  `permanent`, settable from Settings → Deletions. In `trash` mode a `LocalDelete` moves the entity
  to the FreeDesktop trash (`~/.local/share/Trash`, with `.trashinfo` metadata) instead of unlinking
  it. In `permanent` mode the behaviour is byte-identical to today.
- **BREAKING (behavioural, not wire):** an existing config that says nothing gets `trash`. Users who
  relied on local deletes freeing disk space immediately must set `local_delete_mode = "permanent"`.
- The daemon reports the disposal it will actually use for each withheld deletion, and the GUI reads
  severity from that instead of from the direction alone. A trashed local delete is `recoverable`.
- Consequently, in `trash` mode: no `Permanent · this computer` column, no typed-`DELETE` gate, no
  interrupting deletion banner, and no "removes it from this computer for good" copy. The Deletions
  screen shows a local delete beside a remote one, both recoverable, both one click.
- In `permanent` mode every one of those warnings returns unchanged. The friction follows the
  consequence rather than the direction.
- A trash move that fails is a failed item (the #136 partial-pass path). It never falls back to
  unlinking.
- Approval semantics are untouched: `deletion_policy` keeps gating on *direction*, so
  `ask_every_time` and `only_permanent` behave exactly as they do today for every existing config.

## Capabilities

### New Capabilities
- `local-deletion-disposal`: how the engine disposes of a local entity when it mirrors a remote
  deletion — the `trash` / `permanent` choice, where a trashed entity goes, what happens when the
  move fails, how the choice reaches the wire, and which user-facing warnings each mode carries.

### Modified Capabilities
<!-- None. `openspec/specs/` is empty; this change introduces the first capability. The
     delete-approval guard, the plan/apply family and the conflict machinery are unchanged and have
     no spec to amend. -->

## Impact

- **Engine**: `src/config.rs` (new key + `ConfigKey` classification + validation), `src/daemon.rs`
  (the `LocalDelete` executor arm, the pending-deletion payload), a new trash module implementing
  the FreeDesktop spec, `src/ipc.rs` (`PendingDeletion` gains a disposal field).
- **GUI**: `gui/src/js/ui/rows.js` (`severityOf` gains a second input), `screens/deletions.js`,
  `screens/main.js`, `screens/settings.js`, `notifier.js`, `app.js`, `ui/copy.js`, the fixtures for
  each, plus `gui/gui-core/src/config_io.rs` and `gui/src-tauri/src/commands.rs` for the round trip.
- **Dependencies**: one new dependency for the trash move, or an in-crate implementation of the
  ~150-line FreeDesktop spec. Decided in `design.md`.
- **Docs**: `CLAUDE.md` (the module map and the delete-approval invariant), `README.md`,
  `docs/design-v2/05-deletions.md`, `08-settings.md`, `13-copy-deck.md` and `DEVIATIONS.md` — the
  copy changes diverge from the design-v2 frames, and the ledger is how this repo records that.
- **Not affected**: `RemoteDelete` (already recoverable via Proton's Trash), the `Keep` /
  purge-subtree flow, conflict sidecars, and the plan/apply token comparison.
