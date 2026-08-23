## Context

See `proposal.md` — Why. The constraints that actually shape the approach:

- **The executor seam is one arm.** `src/daemon.rs:4141-4176` is the whole of `LocalDelete`:
  `safe_local_path` → `destination.exists()` → `remove_dir_all` / `remove_file` → note → purge the
  baseline row (and descendants for a directory) → consume the approval. Everything about disposal
  fits between the guard and the note.
- **The GUI decides severity from the direction alone.** `severityOf` (`gui/src/js/ui/rows.js:381`)
  is `direction === "remote" ? "recoverable" : "permanent"`, deliberately fail-closed, and is the
  single source for six call sites: the two-column split and the typed gate
  (`screens/deletions.js`), the main-screen deletion band (`screens/main.js:510`), the notifier's
  `isPermanent` (`notifier.js:82`), and the keyboard default plus `keepPermanentDeletions` in
  `app.js:1725,3722`. One rule, six readers — which is what makes this change tractable.
- **A config save requires a daemon restart** (`gui/src-tauri/src/commands.rs`, `write_config`'s own
  comment). So the file and the running daemon can disagree for as long as the user leaves the app
  open, and any GUI that read disposal from the file would draw the wrong warning during exactly
  that window.
- **`ConfigKey::scope` is an exhaustive match with no `_` arm**, and
  `every_file_config_key_is_classified_exactly_once` compares `ConfigKey::ALL` against what
  `FileConfig` serializes, in both directions. A new key cannot be added quietly.
- **`is_sync_state_path` matches the first path component only** (`src/index.rs:1941`), and it is
  hard-coded to `.sync`. It cannot be reused for a trash directory whose name depends on the uid.
- **The engine has eight direct dependencies** and shells out for everything remote. A new
  dependency is a real decision here, not a formality.

## Goals / Non-Goals

**Goals:**

- One place decides disposal, and it is the same place for the executor, the wire and the UI.
- The failure mode of the new path is "the file is still there", never "the file is gone anyway".
- `permanent` mode is the pre-change code path, not a re-implementation of it.
- The GUI's fail-closed severity rule survives gaining a second input.

**Non-Goals:**

- No per-directory `.proton-sync.toml` override for disposal. `dirconfig.rs` is built for the
  approval guard; extending it is a separate change and nothing in the request needs it.
- No trash for the *remote* side — Proton's own Trash already covers it.
- No restore-from-trash UI. The desktop file manager is the restore surface; that is the point.
- No trash quota, retention or emptying. The desktop owns that.
- No change to the plan/apply token, the approval fingerprint, or the `Keep`/purge-subtree flow.

## Decisions

### D1 — `local_delete_mode` as a per-pair key with two values, not a boolean

`local_delete_mode = "trash" | "permanent"`, default `trash`, `KeyScope::Pair`.

*Why an enum and not `permanent_local_deletes = true/false`.* A boolean has no room for a third
disposal and reads ambiguously in a file (`permanent_local_deletes = false` — false meaning what?).
The repo already has this exact shape in `DeletionPolicy`: a named enum with `as_str`, `FromStr`,
`ALL`, and an error that lists the accepted values. Follow it.

*Why per-pair.* `ConfigKey::scope`'s rule is "daemon-wide by nature, daemon-wide by force (the keys
that build the one shared `ProtonDriveClient`), everything else describes a tree". Disposal
describes a tree. Two pairs could reasonably differ.

*Alternative rejected:* reusing `DeletionPolicy`. It answers "which deletions wait for a person",
keyed on direction. Disposal answers "what happens when one applies". Folding two questions into one
enum would give four-way combinatorics with no defensible spellings, and would change what existing
`only_permanent` configs do — which the user explicitly ruled out.

*Alternative rejected:* a CLI flag pair (`--trash-local-deletes` / `--no-...`). A flag can be added
later; the setting is the deliverable. One flag, `--local-delete-mode <MODE>`, is enough for parity
with the other resolved keys.

### D2 — The `trash` crate (v5, MIT), not a hand-rolled spec implementation and not `gio trash`

*Verified:* `trash 5.2.6`, MIT, `rust-version = 1.85` (edition 2024's floor, so no MSRV problem).
Its Linux dependency set is `log`, `libc`, `once_cell`, `scopeguard`, `urlencoding`, plus optional
`chrono`. No D-Bus, no glib, no portal. It implements the two-tier FreeDesktop layout the spec
requires: the home trash under `$XDG_DATA_HOME/Trash`, and `.Trash/$uid` / `.Trash-$uid` at the
mount top directory when the file is on another filesystem
(`freedesktop.rs::execute_on_mounted_trash_folders`).

**Keep the `chrono` feature on.** It is what writes `DeletionDate` into the `.trashinfo` file
(`freedesktop.rs:521-526`); without it the entry lands with no timestamp, and the spec requirement
"the trash records ... the time it was trashed" fails. Turn off the Windows `coinit_*` defaults by
taking `default-features = false, features = ["chrono"]`.

*Why not hand-rolled.* The spec is small to read and full of edge cases that only bite in
production: `%`-encoding the original path, `create_new` collision-renaming inside `files/`,
`info/` written before the rename so a crash cannot orphan a file, top-directory discovery, sticky-
bit validation on `.Trash`. Writing it here means owning all of it plus the tests.

*Why not `gio trash`.* It matches the shell-out pattern, but adds a hard runtime dependency on glib
tooling that a headless or minimal install may not have, and the engine's daemon is exactly the
process most likely to run on such a host. Failure there would be at delete time, per file.

*Escape hatch if the dependency is refused in review:* the crate's Linux backend is one file and the
seam below is a single function, so an in-crate implementation is a drop-in replacement later. The
seam is what matters; the crate is an implementation of it.

### D3 — The seam is `LocalDisposal::apply`, one function, called from one place

A small module (`src/trash.rs`) exposing the mode enum and one operation:

```
fn dispose(mode, absolute_path, entity_kind) -> AppResult<()>
```

`Permanent` runs today's exact two lines. `Trash` calls the crate. The `LocalDelete` arm calls this
and nothing else, so there is no second place that decides how a local entity goes away — the
failure shape this repo files under "two places computing the same thing".

The `destination.exists()` check stays where it is and keeps its current meaning: nothing to
dispose of is success, in both modes.

### D4 — A trash failure is a failed item, and there is no fallback

The `LocalDelete` arm already runs inside `execute_action`, whose `Err` is caught by the #136 path:
the action's queued mutations are rolled back, the path joins `PassFailures`, the pass continues and
ends `Partial`, the event cursor is held, and the path is re-queued into `pending_changes`. So
propagating the crate's error is the entire implementation — and it is the correct one.

**No fallback to `remove_file`.** A fallback would make the trash a best-effort courtesy and the
permanent removal the real behaviour, which inverts the whole point. It is written as a spec
requirement (`A failed trash move is a failed item, never a silent removal`) so it cannot be
"optimised" back in.

The systemic-failure breaker (`CONSECUTIVE_FAILURE_LIMIT` with zero successes) already covers a
trash that is broken for every item — a read-only `$HOME`, a full disk — so a thousand-file
deletion into a dead trash abandons the pass instead of grinding.

### D5 — Disposal crosses the wire per pending deletion, and the GUI never reads the config file for it

`PendingDeletion` gains:

```
#[serde(default)]
pub disposal: LocalDisposal,   // Default::default() == Permanent
```

*Why on the item and not on the status payload.* It is the item's property, it is what the card
draws, and per-item leaves room for a future per-directory override without a wire change. A
`RemoteDelete` reports `Recoverable` — the field says what happens to *this* deletion, so the client
does not need a second rule to combine direction and disposal.

*Two enums, deliberately.* `LocalDeleteMode { trash, permanent }` is the **config**: it names the
mechanism a user chooses. `LocalDisposal { recoverable, permanent }` is the **wire**: it names the
consequence a client draws. They are not the same question — a `RemoteDelete` has a disposal and no
mode — and unifying them would put "which trash" into a field whose readers only care whether the
file can come back. The GUI therefore compares against `"recoverable"`; `"trash"` never crosses the
wire.

*Why `Default = Permanent`.* `#[serde(default)]` is required for wire compatibility, and the default
is what an older daemon's reply parses to. `severityOf` fails closed by asking for the safe value
rather than excluding the dangerous one; the wire default must agree with it, or the fail-closed
rule is undone by the serde attribute. An absent field means "this daemon cannot tell me", and the
cautious reading of that is permanent.

*Why not the config file.* D-context: saving requires a restart, so the file leads the daemon. A GUI
that read the file would drop the typed gate the moment the user picked trash mode — while the
running daemon was still unlinking. The daemon reports what it will do.

### D6 — `severityOf` gains a second input and keeps its shape

```
severityOf(direction, disposal)  →  "recoverable" | "permanent"
```

Recoverable iff `direction === "remote"` **or** `disposal === "recoverable"` — the wire value, not
the config spelling. Everything else, including both arguments missing, is permanent. Same fail-closed rule, same single definition, same six
readers — the call sites change their arguments, not their logic. `screens/deletions.js` keeps its
two columns and its column ordering; in trash mode the permanent column is simply empty, which is a
state the screen already handles (a queue with nothing on one side).

*Alternative rejected:* a `severity` string computed by the daemon and sent whole. It moves a UI
decision into the engine and leaves the GUI unable to explain *why* something is recoverable, which
the copy needs (`Moved to Proton Drive's Trash` vs `Moved to this computer's Trash` are different
sentences).

### D7 — Copy is added, not deleted

The permanent strings stay in `ui/copy.js` — permanent mode still uses every one of them. What is
added is the local-trash voice for the recoverable column: a `recoverableLocal` eyebrow/sub pair
(`Recoverable · this computer` / `Moved to this computer's Trash. You can restore it from your file
manager.`) and a local `travelExplainer` counterpart. `DELETIONS.recoverable*` is currently written
as though recoverable always means Proton, so the column headers become a function of which side the
card is on, not a constant.

*Consequence to plan for:* the fidelity gates compare the built screen against the design-v2 frames,
and the frames draw the old two-column arrangement. This diverges deliberately, and the repo's
mechanism for that is `docs/design-v2/DEVIATIONS.md`. Budget for the ledger entry, the copy-deck
update and the fixtures in the same change, or the gates go red for the right reason.

### D8 — The trash directory inside a sync root is excluded by the scanner, generally

If a pair's `local_root` is itself a mount point, the crate creates `.Trash-$uid` **inside** it and
the scanner would upload every trashed file back to Proton — turning a delete into a round trip.
`is_sync_state_path` cannot cover it: it compares one hard-coded name at the first component.

Add a sibling predicate in `src/index.rs` beside `is_download_scratch_path`, which is already the
"engine-made directory living inside the root" pattern: match a component **byte-exactly** against
`.Trash` and against `.Trash-<digits>`, at any depth (a nested mount point inside the root is a real
case), and wire it into `should_ignore_path` so the scanner, the base-index filter and the watcher
all inherit it — the same three readers `is_download_scratch_path` has.

*Why match the general name and not just this uid's.* A trash directory is not the user's content
under any uid, and matching one uid would leave a root shared between two accounts syncing the other
one's deleted files.

*This is a behaviour change for a path a user could theoretically have synced deliberately.* Name it
in the release notes; it is the same trade already made for `.sync` and the download scratch.

### D9 — Settings puts it on the Deletions tab, under the existing policy cards

The Deletions tab already owns "what happens to deletions" and holds the three `deletion_policy`
radio cards. The disposal control is two more radio cards (`radioCard` is already the tab's
vocabulary) under their own section title, with its own `keyLine("local_delete_mode")`. The two
controls are adjacent and independent, which matches what they are: one decides *whether you are
asked*, the other decides *what happens when it goes ahead*.

Round trip: `ConfigDoc::get_local_delete_mode` / `set_local_delete_mode` in
`gui/gui-core/src/config_io.rs` (plain key, no dual-spelling problem — unlike `deletion_policy` this
key has exactly one form), a field on `ConfigPayload` and on `ConfigUpdate`
(`gui/src-tauri/src/commands.rs`), and a handler on the tab.

## Risks / Trade-offs

- **Trash mode makes deletions cost disk space, silently.** A user syncing a folder they routinely
  purge now fills their trash instead of freeing space. → The permanent card is one click away and
  the setting is on the tab a user goes to for exactly this. Name the trade in the card's body copy
  rather than hiding it.
- **Removing the typed-`DELETE` gate removes friction the product was built around.** If the trash
  move silently no-ops on some system, a one-click approval now destroys data that used to require
  typing a word. → D4 makes a failed move a failed item rather than a removal, so "silently no-ops"
  is not reachable through the engine. Cover it with a test that points the disposal at a trash it
  cannot write to and asserts the file survives *and* the pass reports it.
- **`.Trash-$uid` exclusion could hide a directory a user genuinely syncs.** → Vanishingly rare, and
  the alternative is a delete loop that re-uploads everything the user deleted. Documented.
- **A new dependency in a crate that has eight.** → MIT, no transitive D-Bus or glib, and D2 records
  the escape hatch: the seam is one function and the crate's Linux backend is one file.
- **Six GUI call sites and a fidelity gate that compares against frames drawn for the old
  arrangement.** → The DEVIATIONS ledger is the repo's own mechanism; the tasks list the call sites
  individually so none is found later by a gate instead of by the change.
- **A daemon older than the GUI reports no disposal and every local deletion draws as permanent.**
  → Correct and intended (D5). It is the same reading `severityOf` already gives an unknown
  direction.

## Migration Plan

1. **No migration step for existing configs.** A file that says nothing gets `trash`. Nothing is
   rewritten; the key is absent until a user sets it, exactly like every other defaulted key.
2. **The default changes behaviour on upgrade**, and that is the deliverable rather than an
   accident. Release notes must say: local deletions now go to your trash; set
   `local_delete_mode = "permanent"` to restore the previous behaviour.
3. **Rollback** is `local_delete_mode = "permanent"` in the config, which restores the exact prior
   code path (D3), with no data to migrate back — the trashed files are still in the trash.
4. **Ordering within the change:** the engine key + disposal seam + wire field can land and be
   correct on their own (the GUI simply keeps drawing permanent, which an absent field already
   means). The GUI severity and copy work depends on the wire field existing. Tasks are ordered
   accordingly.
