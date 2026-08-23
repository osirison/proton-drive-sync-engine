## 1. The disposal seam

- [x] 1.1 Add `trash = { version = "5.2.6", default-features = false, features = ["chrono"] }` to the
      root `Cargo.toml` and confirm `cargo tree -p proton-drive-sync-engine -i trash` pulls in no
      D-Bus/glib crate (design D2 — the whole justification for the dependency is its Linux dep set)
- [x] 1.2 Create `src/trash.rs` with `LocalDeleteMode { Trash, Permanent }` — `Default = Trash`,
      `as_str`/`FromStr`/`ALL`, serde `rename_all = "snake_case"`, `Display`, modelled on
      `config::DeletionPolicy` (`src/config.rs:41-120`)
- [x] 1.3 Add `dispose(mode, absolute_path, entity_kind) -> AppResult<()>` in the same module:
      `Permanent` runs today's `remove_dir_all`/`remove_file` verbatim, `Trash` calls the crate.
      This is the **only** place either happens
- [x] 1.4 Unit-test `dispose` in both modes over a file and over a non-empty directory, plus the
      trash-unavailable case (point `XDG_DATA_HOME` at a path that cannot be created): assert the
      entity **still exists** and an `Err` comes back.
      **The crate reads the environment internally**, `std::env::set_var` is `unsafe` under edition
      2024 and cargo runs tests threaded — so every test that redirects `XDG_DATA_HOME` must
      serialize that mutation or run the disposal in a child process. Settle this in 1.4; tasks 3.2
      and 3.3 inherit whichever mechanism it establishes
- [x] 1.5 Declare the module in `src/lib.rs` and re-export `LocalDeleteMode`

## 2. The config key

- [x] 2.1 Add `local_delete_mode: Option<LocalDeleteMode>` to `FileConfig` and the matching field to
      `DaemonConfigInput`, plus `--local-delete-mode <MODE>` on `proton-syncd`
- [x] 2.2 Add `ConfigKey::LocalDeleteMode`, extend `ALL` to 23, add its `spelling()` arm, and
      classify it `KeyScope::Pair` in the exhaustive `scope()` match (design D1)
- [x] 2.3 Carry it onto `PairConfig` through `DaemonConfigInput`/`FileConfig` merge and
      `DaemonConfig::into_parts`, defaulting to `Trash` when nothing sets it
- [x] 2.4 Validate an unrecognised value in `validate_file_config_text`, with an error naming the key
      and listing both accepted spellings — match the shape `deletion_policy`'s error already uses
- [x] 2.5 Confirm `every_file_config_key_is_classified_exactly_once` and
      `the_key_set_a_classification_guard_reads_keeps_a_field_left_unset` still pass, and add a test
      that a config saying nothing resolves to `Trash`
- [x] 2.6 Add a `resolve_pairs` unit test that two `[[pair]]` tables may set the key to different
      values — it must run at that layer, because `refuse_unsupported_pair_count` rejects any
      two-pair config before the daemon ever sees one

## 3. The executor

- [x] 3.1 Replace the `remove_dir_all`/`remove_file` block in the `SyncAction::LocalDelete` arm
      (`src/daemon.rs:4155-4162`) with a `trash::dispose` call reading the pair's mode. Leave the
      `destination.exists()` early-out, the `safe_local_path` guard, the pass-log note, the baseline
      purge and the approval consumption exactly as they are
- [x] 3.2 Daemon test: a remote deletion in the default (trash) mode leaves the file out of the sync
      root, present in a redirected `XDG_DATA_HOME` trash, and its baseline row purged
- [x] 3.3 Daemon test: the same deletion in `permanent` mode removes the file and puts nothing in
      the trash — the pre-change behaviour, asserted rather than assumed
- [x] 3.4 Daemon test: a failing trash move leaves the file on disk, reports it in `failed_items`,
      leaves the baseline row **unpurged**, holds the event cursor, lets the following actions in
      the plan run, and ends the pass `Partial` (design D4 — this is the invariant the whole
      warning removal rests on)
- [x] 3.5 Daemon test: a directory deletion in trash mode moves the whole tree as one entity and
      purges the descendant baseline rows

## 4. The trash directory is never synced

- [x] 4.1 Add `is_trash_dir_path` beside `is_download_scratch_path` (`src/index.rs:1955`): byte-exact
      component match on `.Trash` and on `.Trash-<digits>`, at any depth
- [x] 4.2 Wire it into `should_ignore_path` so the scanner, the base-index filter and the watcher all
      inherit it — the same three readers `is_download_scratch_path` has
- [x] 4.3 Test: a `.Trash-1000/files/doomed.txt` under the local root is neither scanned nor planned
      for upload, and a `.Trash-1000` created mid-run is not queued by the watcher
- [x] 4.4 Test: a *user* directory called `Trash` or `.Trashcan` is still synced — the predicate must
      not over-match

## 5. The wire

- [x] 5.1 Add `#[serde(default)] pub disposal: LocalDisposal` to `ipc::PendingDeletion`, with
      `Recoverable`/`Permanent` variants and `Default = Permanent` (design D5 — the default must
      agree with `severityOf`'s fail-closed rule)
- [x] 5.2 Populate it in `decide_delete_gate`: a `RemoteDelete` is always `Recoverable`; a
      `LocalDelete` is `Recoverable` iff the pair's mode is `Trash`
- [x] 5.3 Test: a reply serialized without the field parses back as `Permanent`, and a full round
      trip preserves both values
- [x] 5.4 Render the disposal in `proton-sync status`'s deletions section so the CLI says which
      deletions are recoverable, and add the assertion to `tests/ipc_cli.rs`

## 6. GUI severity and screens

- [ ] 6.1 Change `severityOf(direction)` → `severityOf(direction, disposal)` in
      `gui/src/js/ui/rows.js:381`: recoverable iff `direction === "remote"` **or**
      `disposal === "recoverable"` (the **wire** value — `"trash"` is the config spelling and never
      crosses the wire), everything else permanent. Rewrite the doc comment's fail-closed
      paragraph to cover both inputs — it currently argues about one
- [ ] 6.2 Update all six call sites to pass the item: `screens/deletions.js` (the column split at
      :93, the gate at :128/:225/:307, the recoverable branch at :158), `screens/main.js:510`,
      `notifier.js:82`, `app.js:1725` and `app.js:3722`
- [ ] 6.3 `screens/deletions.js`: the column eyebrow and its sub-line become a function of the card's
      side, not a constant — a recoverable *local* card says `Recoverable · this computer`, a
      recoverable *remote* one keeps `Recoverable · Proton Drive`
- [ ] 6.4 Verify the empty-permanent-column state renders: in trash mode with only local deletions
      queued, one column is empty and the screen must not draw a headerless or half-width layout
- [ ] 6.5 `notifier.js`: confirm the deletion trigger goes silent when every queued deletion is
      recoverable, and that `only_permanent_deletions` policy still lets a genuine permanent one
      through

## 7. GUI copy

- [ ] 7.1 Add to `ui/copy.js` `DELETIONS`: `recoverableLocal`, `recoverableLocalSub`
      (`Moved to this computer's Trash. You can restore it from your file manager.`), a local
      counterpart to `travelExplainer`, and a `toTrashLocal` button label
- [ ] 7.2 Leave `permanent`, `permanentSub`, `fileConsequence`, `folderConsequence*` and
      `typeToDelete` in place — permanent mode still draws every one of them (design D7)
- [ ] 7.3 Add the two settings cards' copy: titles, bodies, and the disk-space trade named plainly in
      the trash card's body (see Risks)
- [ ] 7.4 Update `docs/design-v2/13-copy-deck.md` with every string added or repurposed

## 8. Settings round trip

- [ ] 8.1 `gui/gui-core/src/config_io.rs`: `get_local_delete_mode` / `set_local_delete_mode`, plain
      single-spelling key, re-exporting the engine's enum rather than copying it
- [ ] 8.2 `gui/src-tauri/src/commands.rs`: field on `ConfigPayload` (populated in `read_config`) and
      on `ConfigUpdate` (applied in `write_config`)
- [ ] 8.3 `gui/src/js/screens/settings.js`: a second section on the Deletions tab — two `radioCard`s
      under a section title, with `keyLine("local_delete_mode")`, and the same
      "no card until the file has been read" rule the policy cards use
- [ ] 8.4 Wire the handler through `app.js` to `api.js`'s config write, and confirm the
      restart-required prompt fires as it does for the other keys
- [ ] 8.5 Extend `gui/src/js/fixtures/settings.js` and `fixtures/deletions.js` with both modes, and
      add a fixture whose queue mixes a trashed local deletion with a remote one

## 9. Gates, docs and the ledger

- [ ] 9.1 `cargo fmt --all -- --check`, `cargo clippy --workspace --all-targets --all-features -- -D
      warnings`, `cargo test --workspace --all-targets --all-features`
- [ ] 9.2 `(cd gui && npm run check)` and the fidelity/copy gates; record every intentional frame
      divergence in `docs/design-v2/DEVIATIONS.md` with its reason (design D7)
- [ ] 9.3 Update `docs/design-v2/05-deletions.md` and `08-settings.md` to describe the two modes
- [ ] 9.4 Update `CLAUDE.md`: the `src/trash.rs` module line, the `local_delete_mode` key in the
      `src/config.rs` paragraph, the trash-directory rule in the `src/index.rs` paragraph, and the
      delete-approval invariant's note that disposal is a separate question from gating
- [ ] 9.5 Update `README.md`'s config reference and add the upgrade note: local deletions now go to
      the trash; `local_delete_mode = "permanent"` restores the old behaviour
- [ ] 9.6 Manual check on a real desktop: delete a synced file on Proton, approve it, then confirm
      the file appears in the file manager's Trash and restores to its original path
