# Design note — event-driven reconcile (#21)

- **Status:** Implemented & verified (opt-in `events_driven`, default **off**)
- **Date:** 2026-07-21 (design); verified 2026-07-23
- **Issue:** #21 · Epic #16 · builds on #19 (cursor + `path_for_proton_id` + `events::node_uid`),
  #20 (`EventsClient`, `CliKeyringSession`, `CurlHttpTransport`) · ADR `docs/adr/0001-*`

## Implementation status (2026-07-23)

Both stages landed together in `src/reconstruct.rs` (pure `reconstruct_remote` = `base ⊕ delta`,
Stage 2's "reconstruct-and-plan"), `src/daemon.rs` (bootstrap vs `try_incremental_reconcile`
split, dedicated events-poll `select!` arm), `src/events.rs` (`EventSource`), `src/proton.rs`
(`list_directory`), plus the `events_driven` / `events_full_scan_every` config. Verified in strict
order before enabling:

1. **build / clippy / full test suite** — green (incl. `reconstruct` unit tests + daemon
   fake-`EventSource` tests).
2. **Live id-identity HARD GATE** (`tests/events_identity_live.rs`) — passed read-only **and** the
   write round-trip: a stored composed `proton_id` equals `node_uid(volume, event.LinkID)` for the
   same node against the real account. The stream-only design's core identity holds.
3. **Adversarial spine review** — found and fixed the **startup-floor regression** (below); the
   commit-after-side-effects, cursor-before-snapshot, idle-skip, and off-is-identical invariants
   otherwise hold.
4. **Flag-on live e2e** (`daemon::tests::live_event_driven_reconcile_*`, `#[ignore]`) — a real
   remote create was discovered from the event stream alone, resolved via **one targeted directory
   listing**, and downloaded with **zero** full-tree walks beyond the bootstrap; the cursor advanced.

### Fix applied during verification — startup floor

The first reconcile *after process startup* must full-scan, not go incremental: a fresh process has
an empty `pending_changes` (`notify` never replays pre-existing files), so an incremental pass would
idle-skip a file edited **while the daemon was down** until the K-floor. `incremental_passes_since_full_scan`
is now seeded at `events_full_scan_every` so the first pass snapshots ("get truth"), then streams.
Guard: `daemon::tests::first_reconcile_after_startup_full_scans_even_with_a_persisted_cursor`.

### Known limitations (filed as follow-ups, none is data loss)

- **#30** — a nested `Created` whose indexed-parent listing lags returns `Ok(None)` → dropped +
  cursor-advanced → deferred to the K-floor (bounded latency). Top-level creates re-anchor instead;
  the two paths should be made consistent.
- **#31** — a node listed but trashed/deleted before its download fails the whole reconcile pass
  (TOCTOU); self-heals next pass but should be a per-node skip.
- **#32** — the incremental gate derives the volume from *base* records, so an all-Proton-native (or
  not-yet-recorded) remote can't engage the gate and safely stays on full walks even though a cursor
  exists.

## Goal

Stop paying an **O(folders)** full remote walk (`proton::list_entities_or_missing_root`, one
`filesystem list` per directory) on every reconcile. Use the volume event stream to learn what
changed and do less work.

## The core constraint (why this isn't "just pass the delta to the planner")

`sync::plan_sync_entities` is a **pure planner over three _full_ maps** — `local_entities`,
`remote_entities`, `base_index` — and it unions all their paths. It treats *"a path is in
`base_index` but absent from `remote_entities`"* as a **remote deletion**. So feeding it a
**partial** remote map (only the changed nodes) would make every *unchanged* remote file look
deleted → mass destructive plan. Any incremental design must therefore either (a) present the
planner a **complete, correct** remote view, or (b) **scope** the planner to only the affected
paths. This constraint drives the two-stage plan below.

A second fact from #20/#19: node events carry **ids, not names/paths** (`nodeUid`,
`parentNodeUid`), and the name is encrypted. Turning an event into a planner input requires
resolving ids → paths (via `index::path_for_proton_id`, bridging the raw `LinkID` with
`events::node_uid`) and, for a *newly created* node, a **targeted** `filesystem list` of its
parent to learn its name + SHA-1 (the CLI decrypts). This is O(changes) work, not O(folders) —
but it is real work with real edge cases.

## Approach: two stages

Land a correct, low-risk win first; take on the delta→planner complexity second, behind the
same structure.

### Stage 1 — event-gated full reconcile (this PR)

**Idea:** keep the existing full reconcile and its full planner *exactly as-is*; use events only
to decide **whether to run it**. Instead of walking the whole tree every scan interval, walk
only when the event stream (or the local watcher) says something changed — with a **forced
full-scan floor** so a missed signal can never strand a change indefinitely.

Per periodic tick (the `interval.tick()` arm; the fs-watch and `syncnow` arms already fire only
on demand):

1. If event-driven mode is **disabled**/unavailable (no session, no volume id, config off) →
   behave exactly as today (full reconcile). *No behavior change unless opted in.*
2. **Forced full-scan floor (backstop — see below).** If this is the **first reconcile since
   process startup**, or **K ticks have been skipped** since the last full scan → run a full
   reconcile regardless of the delta, refreshing the cursor snapshot-first (step 3, no-cursor
   path). Reset the skip counter.
3. Otherwise, cursor-driven. **Always snapshot the cursor _before_ the reconcile's listing**
   (invariant below):
   - **No cursor** (first run) → snapshot `s = EventsClient::latest_cursor`, full reconcile,
     store `s`. Bootstrap.
   - Fetch `page = EventsClient::events_since(volume, cursor)` (this happens *before* any
     listing, so `page.latest_event_id` already satisfies the snapshot-first invariant):
     - **error / auth failure** → full reconcile (never skip on uncertainty); leave the cursor.
     - **`refresh == true`** → snapshot `s = latest_cursor`, full reconcile, store `s` (server
       asked us to discard our position).
     - **non-empty `changes`** → full reconcile; store `page.latest_event_id`; reset skip counter.
     - **empty `changes`** *and* `pending_changes.is_empty()` → **skip the walk**; store
       `page.latest_event_id`; **increment the skip counter**. The win: idle remote + idle local
       = zero `filesystem list` calls (until the floor forces one).
     - empty `changes` but pending local changes → full reconcile; store `page.latest_event_id`;
       reset skip counter.

**Cursor lifecycle & the snapshot-first invariant.** The stored cursor must always be at a
position **≤ the reconcile listing's observation time**, or a change landing between the listing
and a later cursor read would be seen by neither this listing nor any future delta — silent
loss. The normal path satisfies this for free (`events_since` runs before the listing). The
bootstrap/refresh paths must therefore snapshot `latest_cursor` **before** reconciling, not
after. The cursor advance happens **inside the existing post-side-effects commit** (`daemon.rs:767`
— `store_event_cursor(&transaction, …)` joins the same transaction as the index mutations); a
failed reconcile does not advance it (events replay next pass — idempotent; skipping would be
loss). The skip case advances the cursor in its own tiny transaction.

**Backstop (in Stage 1, not deferred).** The skip condition keys off `pending_changes`, which is
populated by the *same fallible watcher* a periodic scan exists to backstop — so "delta empty +
pending empty" does **not** prove "a full reconcile would be a no-op." The decisive case is a
**restart**: daemon stops → user edits a local file → daemon starts with the cursor persisted,
remote delta empty, `pending_changes` empty (fresh process) → a naive skip would never sync that
edit. Today's unconditional periodic reconcile catches it, so Stage 1 must too. Hence the forced
floor in step 2: **a full scan on the first reconcile after startup, and after every K skipped
ticks.** This keeps the win (full scan every K intervals + on events, instead of every interval)
while removing the regression. #22 layers the *richer* reconvergence semantics (anchor
invalidation, cursor-age policy) on top; the skip-counter floor itself is intrinsic to Stage 1.

**Why this is safe.** The planner still runs on a full, freshly-scanned view whenever it runs,
so a plan is never wrong. Events (plus the floor) only ever cause us to *skip* a reconcile — and
a skipped change is picked up by the next event, the next local watch event, or the forced floor
within K ticks. Stage 1 does **not** use id→path resolution, so it does not depend on the
id-identity gate.

**Config.** Add an opt-in `events_driven` flag (default **off**) + `events_full_scan_every`
(the skip floor **K**, default a small number) + the volume id source (derive from the remote
listing's `treeEventScopeId`, or configure). Off = today's behavior verbatim.

### Stage 2 — true incremental planning (follow-up PR)

Turn the delta into a **scoped** plan so a single change costs O(1) `filesystem list` calls, not
a full walk. Two candidate shapes; the design note recommends validating (b) but implementing
the smaller (a) first:

- **(a) Reconstruct-and-plan (index as remote mirror).** Build a *full* `remote_entities` from
  the index (each synced record with a `proton_id` → a `RemoteEntity`; the record's SHA-1 is the
  last-synced remote content), then **apply the delta**: `Deleted`/trashed → remove; `Created` →
  targeted-list the parent for name+SHA-1 and add; `Updated` → targeted-list for the new SHA-1
  and update. Run the **existing full planner** on `(local, reconstructed_remote, base)`. This
  preserves all planner semantics (no false deletions) without a full walk. Cost: reconstructing
  the mirror is O(index) in memory (cheap) + O(changes) targeted lists.
- **(b) Affected-paths scoping.** Restrict the planner's universe to only the changed paths.
  Simpler per-event, but the planner's directory-deletion verdicts and cross-path logic assume a
  full view, so this is riskier for directories/moves — likely not worth it over (a).

**Hard parts (why Stage 2 is its own PR):**

- **The id-identity gate (must pass first, live).** Stage 2 relies on `reconcile-written
  proton_id == events::node_uid(volume, that node's event LinkID)` for the *same* node. #19
  verified the *formula*; the full round trip is unverified. Gate Stage 2 on a live test: sync a
  real file → capture its event → bridge → assert `path_for_proton_id` resolves. Also confirm
  production listings populate `node.uid` (composed), not `node.id` (raw) — `proton.rs` prefers
  `id`, and a raw id there would silently break the bridge (→ always fall back).
- **Moves** arrive as `Updated` with a changed `parentNodeUid`; detecting a move needs the old
  parent from the index. Until handled, treat an unresolved/moved node as a fallback trigger.
- **Unresolvable ids** (a `Created` whose parent isn't in the index yet; a `proton_id` still
  `None` because the node was just uploaded and not yet listed) → fall back to a full scan for
  that pass. Never guess.
- **Selective-sync filters** (`ScanOptions`) must be applied to the delta and to targeted
  listings exactly as they are to full listings.

## Invariants preserved (both stages)

- **Commit-after-side-effects / no partial commit.** Unchanged: the cursor advance is part of
  the same single post-success transaction as the index mutations.
- **Path-safety.** Every path derived from an event or a targeted listing passes
  `validate_relative_path` / `safe_local_path` before use (Stage 2).
- **Non-destructive on unknown digests / conservatism.** A `refresh`, an error, an unresolved
  id, or a move all trigger the full-scan fallback rather than a guessed destructive action.
- **Selective-sync everywhere.**

## Testing

- **Stage 1:** inject a fake `EventsClient` seam into the daemon (mirror the existing fake
  `ProtonClient` pattern). Cover: no-cursor bootstrap; empty delta + idle → skip (assert zero
  `list` calls via the fake `ProtonClient` call counter); empty delta + pending local → reconcile;
  non-empty delta → reconcile + cursor advanced; events error → reconcile + cursor unchanged;
  `refresh` → reconcile + cursor reset; reconcile failure → cursor **not** advanced (extend the
  existing no-partial-commit fake). **Backstop:** first reconcile after startup always full-scans
  even with an empty delta + persisted cursor (the restart-regression case); **K** consecutive
  empty-delta ticks force a full scan (assert a `list` fires on the K-th). **Snapshot-first:** the
  stored cursor equals the value read *before* the listing (bootstrap/refresh snapshot ordering).
- **Stage 2:** the live id-identity gate above, plus fake-delta planning tests
  (create/update/delete/move → correct scoped plan; unresolved id → fallback).

## Recommendation

Implement **Stage 1** now (small, correct, big idle-cost win, exercises all the plumbing —
cursor load/store/advance, `EventsClient` wired into the daemon, fallback matrix — with no
dependence on id resolution). Defer **Stage 2** to a focused follow-up gated on the live
id-identity check. Keep `events_driven` **off by default** until Stage 1 is proven on a real
account.
