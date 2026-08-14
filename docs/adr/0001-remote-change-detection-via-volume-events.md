# ADR 0001 — Reach Proton Drive change events via an SDK sidecar

- **Status:** Proposed
- **Date:** 2026-07-21
- **Issue:** #18 (spike + ADR) · Epic #16 · Related: #23 (CLI not concurrency-safe)
- **Supersedes framing of:** #17 (parallel `filesystem list` BFS — deferred)

> **Amended 2026-07-21 (#20 spike) — see [Addendum](#addendum-2026-07-21--event-detection-is-auth-only-20).**
> The cost analysis below overstated auth: **detecting** the change delta needs only a Proton
> *session*, not the `account`/unlocked-keys/PGP the full SDK client demands. The decision
> (use the volume events API) stands; the implementation vehicle is now more open.

## Context

The engine reconciles the remote tree by walking it with the `proton-drive` CLI —
one `filesystem list --json` per directory, **O(folders)** network round trips every
scan (`proton::list_entities_or_missing_root`). Prior investigation (see the epic)
established:

- There is **no parent-level "changed" flag** on `filesystem list`/`info`; a nested
  change leaves every ancestor byte-identical, so subtrees cannot be pruned.
- Proton's own clients avoid full walks with a **volume-level event stream**: fetch a
  cursor, then poll deltas since that cursor — **O(changes)**, not O(folders).
- The `proton-drive` CLI wraps this internally but **exposes no `events` command**, and
  it is **not concurrency-safe** (shared SQLite → `SQLITE_BUSY`; see #23), which killed
  the CLI-based parallel-list idea (#17).

This ADR decides **how the engine reaches that event stream**. Everything below (#19–#22)
depends on this decision and on the schema it fixes.

### What we confirmed (live, 2026-07-21)

- **The event API is real and live.** The CLI's own log shows
  `GET https://drive-api.proton.me/drive/v2/volumes/{volumeId}/events/{cursor}` returning
  `200`, polled ~every 30s, plus `GET .../core/v5/events/{coreCursor}` for account-level events.
- **The cursor is a simple per-volume value.** The CLI persists it at
  `~/.local/share/proton-drive-cli/events.json`:
  ```json
  { "version": 1,
    "drive": { "core":     { "lastEventId": "<core cursor>" },
               "<volumeId>": { "lastEventId": "<volume cursor>" } },
    "photos": {} }
  ```
  The `<volumeId>` equals the `treeEventScopeId` already present on every node we list.
- **The cursor reacts to a change.** After creating `/my-files/zzz-evt-probe`, the live
  subscriber's persisted volume cursor advanced `2Qmu…→82fX…` within ~9s — *consistent with*
  the mutation producing a `node_created` event. (Caveat: no negative control was run and a
  live subscriber ticks core events independently, so this corroborates the log/`.d.ts`
  evidence rather than proving causation on its own.)
- **The SDK is public.** `@protontech/drive-sdk` (v0.19.2) is on npm and exposes exactly
  the primitive we need (below).

## Decision

**Reach the event stream through a long-lived Node/Bun sidecar process built on
`@protontech/drive-sdk`.** The Rust daemon talks to it over a line-delimited JSON
protocol (stdout/stdin or a Unix socket). The sidecar owns one authenticated SDK client
and its caches; the daemon never shells the CLI concurrently.

The core SDK call:

```ts
// treeEventScopeId == volumeId (read off node metadata); lastEventId == our stored cursor.
for await (const ev of client.iterateEvents(treeEventScopeId, lastEventId, signal)) { … }
// iterateEvents is a PULL AsyncGenerator<DriveEvent>; when lastEventId is omitted it
// starts by emitting a FastForward carrying the current latest event id.
```

We use the **pull** API (`iterateEvents`), **not** the push/subscription APIs
(`subscribeToTreeEvents`, `getEventScheduler`, `subscribeToDriveEvents`), which are
documented "only one instance of the SDK should subscribe." A cursor-driven pull with our
own persisted `lastEventId` is stateless on the server side and coexists with the user's
CLI/desktop app, provided the sidecar uses its **own** cache directories (see Consequences).

## Options considered

| Option | Verdict | Why |
|---|---|---|
| **(a) Upstream CLI `events` subcommand** | Rejected | Doesn't exist; depends on Proton's roadmap; and #23 shows shelling short-lived CLI processes is concurrency-fragile even if it did. |
| **(b) `@protontech/drive-sdk` sidecar** | **Chosen** | One long-lived process owns the cache (warm, no `SQLITE_BUSY` contention) and is where events + E2E decryption already live. Public, typed API (`iterateEvents`). |
| **(c) Direct Drive HTTP from Rust** | Rejected | Event payloads are E2E-encrypted; would mean reimplementing Proton auth + PGP/key derivation in Rust. Prohibitive and fragile. |
| **(d) Piggyback on the CLI's `cache-entities.sqlite` / `events.json`** | Rejected | Undocumented internal schema; still the shared-SQLite concurrency hazard of #23; couples us to CLI internals that change without notice. |

## The delta schema (authoritative, from the SDK types)

`iterateEvents` yields `DriveEvent` — a discriminated union on `DriveEventType`:

```ts
type DriveEvent =
  | { type: 'node_created' | 'node_updated';           // NodeEvent (CRU)
      nodeUid: string; parentNodeUid?: string;
      isTrashed: boolean; isShared: boolean;
      treeEventScopeId: string; eventId: string }
  | { type: 'node_deleted';
      nodeUid: string; parentNodeUid?: string;
      treeEventScopeId: string; eventId: string }
  | { type: 'fast_forward';  treeEventScopeId: string; eventId: string }   // cursor jump / initial
  | { type: 'tree_refresh';  treeEventScopeId: string; eventId: string }   // FULL REFRESH required
  | { type: 'tree_remove';   treeEventScopeId: string; eventId: 'none' }   // volume/scope gone
  | { type: 'shared_with_me_updated'; treeEventScopeId: 'core'; eventId: string };
```

The list-level wrapper (`EventsListWithStatus`) carries `{ latestEventId, more, refresh, events }`;
`iterateEvents` abstracts `more` (pagination) and surfaces `refresh` as a `tree_refresh` event.

**Critical property:** node events key on **`nodeUid` / `parentNodeUid`, not paths.** A
`node_deleted` carries *only* `nodeUid`. Turning an event into a planner action therefore
requires a **uid → path** mapping the engine must maintain itself.

## Consequences

### Positive
- Remote change detection becomes **O(changes)**: one cursor read + a short delta, instead
  of a full BFS every scan. Deletes/moves/updates arrive explicitly and decrypted.
- The daemon stops shelling `proton-drive` concurrently, sidestepping #23 for the hot path.
- Full BFS remains only as the periodic **reconvergence** safety net (#22), not the hot path.

### Negative / risks
- **Auth bootstrap is the dominant cost, and it is *not* off-the-shelf** (resolved below).
  The `ProtonDriveClient` constructor demands a caller-provided `httpClient`
  (`fetchJson`/`fetchBlob`; the caller injects Proton session headers and handles
  401→token refresh) **and** an `account` exposing **unlocked private keys**
  (`getOwnAddresses()` returning `PrivateKey`s), plus `srpModule`/`openPGPCryptoModule`.
  There is **no "pass a token and go"** path.
- **Runtime dependency:** ships a Node/Bun sidecar + `@protontech/drive-sdk` alongside the
  Rust binaries. New build/packaging surface.
- **Coexistence:** safe only if the sidecar uses its **own** `XDG_CACHE_HOME` (and state)
  so it never shares `cache-*.sqlite` with a user's CLI/desktop app (the #23 failure mode).
  Using pull `iterateEvents` (not `subscribe*`) avoids the "single subscriber" constraint.

### Auth supply surface (resolved for this ADR)

Checked what is actually wireable, because it sets the true cost and could reorder the options:

- **Published (usable directly):** `@protontech/crypto` (v2.1.1) provides the PGP module
  (`OpenPGPCryptoWithCryptoProxy`) the constructor needs. `@protontech/drive-sdk` itself is
  published.
- **NOT published (the blocker):** the **session + account layer** —
  `@proton/account`, `@proton/srp`, `@proton/shared`, and `@protontech/account` are all
  absent from npm. These are what implement SRP login, session/token refresh, and address
  **key unlock** — exactly the `httpClient` + `account` the SDK delegates. The SDK README
  passes both as pre-built inputs and gives **no** guidance on constructing them.

**Consequence:** building the sidecar's `httpClient` + `account` is **real engineering, not
package glue.** The implementations exist but only as (i) Proton's open-source web monorepo
(**GPLv3** — a licensing consideration for vendoring into this repo) and (ii) the CLI's own
bundle (`incubating/account/js`: `auth.ts`, `srp.ts`, `driveAccountAdapter.ts`).

This reorders the auth approach for #20, in preference:
1. **Reuse the CLI's session** from the OS keyring (`libsecret`/`keytar` — confirmed the
   store) and adapt it into `httpClient`/`account`. Now the **primary** candidate: lowest
   new-auth-code, and it lets the sidecar ride the CLI's existing `auth login`. Risk:
   undocumented keyring/session format and unlocked-key handling.
2. **Own session** — vendor/port SRP + session + key-unlock from Proton's GPLv3 web source
   (or the CLI bundle). Highest effort + a GPLv3 licensing decision; the fallback if (1) is infeasible.
3. **Escape hatch worth tracking:** ask Proton to publish an account adapter, or to add a
   `proton-drive events` subcommand (reopens option (a)) — either collapses this cost.

**#20's first task is to prove candidate (1) end-to-end** (keyring session → one live
`iterateEvents` call). If it fails, the effort/licensing profile jumps to (2), which is worth
knowing before #21/#22 are scheduled.

## Status 2026-07-21 — detection core built; own-session = session forking

Two concrete outcomes from the #20 work:

**1. The detection core is implemented and verified (`src/events.rs`).** A pure,
transport-agnostic `EventsClient<T: HttpTransport, S: SessionProvider>` fetches
`.../events/latest` + `.../v2/volumes/{id}/events/{cursor}` and normalizes the cleartext
delta into `RemoteChange` values (created / updated / deleted, with a `trashed` flag —
recording the subtlety that a **trash arrives as `Updated` + `trashed=true`, not `Deleted`**).
It refreshes once on `401`. Verified by unit tests against real-shaped fixtures **and** an
`#[ignore]` live test (`tests/events_live.rs`) that ran green against the real API through a
temporary session-reuse harness. The crate stays networking-dependency-free — the concrete
transport/session are injected.

**2. "Own login" resolves to session *forking*, not SRP.** Rather than reuse the CLI's tokens
(which couples the two over refresh-token rotation) or re-implement SRP+2FA+CAPTCHA (heavy,
unverifiable, wrong `x-pm-appversion`), the engine will mint an **independent** session by
forking the CLI's existing login: `POST /auth/v4/sessions/forks` → `GET
/auth/v4/sessions/forks/{selector}` (endpoints confirmed present in the CLI). A forked session
has its own UID/tokens and its own refresh lifecycle — the clean, decoupled architecture — and
needs no password/CAPTCHA. It is HTTP + symmetric crypto only (no PGP), so it stays
Rust-native and drops in behind `SessionProvider`.

**Remaining before #21 (daemon wiring):**
- Name resolution for newly-created nodes (`nodeUid`→name) remains the one place decryption is
  still required (SDK/CLI), per the addendum.

## Status 2026-07-21 (cont.) — session ownership: reuse now, independent login deferred

Investigating the independent-session ("own login") build surfaced a decision-changing fact:
**Proton has no headless "fork from the CLI's local session" path.** The CLI's `auth login`
is a *browser* flow — it calls the unauthenticated `sessionForksInit` (`GET
/auth/v4/sessions/forks`), opens `account.proton.me` with its app id (`this.authClientId`) +
`UserCode`, then **polls** `sessionForksStatus` until the user signs in, and consumes the
fork. The CLI is a fork *consumer*; the *producer* is an authenticated web session. So a truly
independent session **requires an interactive browser login** (and, lacking our own registered
client id, would use the CLI's app identity) — it cannot be minted headlessly.

Given that, the choice was taken to the user, who chose **reuse now, independent login later**:

- **Landed:** `src/session.rs` — `CliKeyringSession` (reuses the CLI's keyring session,
  read-only; `refresh` re-reads the keyring rather than owning a token refresh) + a dependency-
  free `CurlHttpTransport`. Verified end-to-end via the live test. This unblocks #21/#22.
- **Accepted limitation:** the daemon is only as fresh as the CLI keeps the session — if the
  CLI is idle and the token expires, a pass `401`s and is skipped until the CLI refreshes.
  `proton-drive` is, for now, the auth owner by design.
- **Deferred (own login):** an independent session provider that replicates the browser fork
  flow (init → browser → poll → consume/decrypt), dropped in behind the same `SessionProvider`
  trait. Its cost/robustness (interactive-only, reverse-engineered fork crypto, CLI app id) is
  why it is a later, isolated piece rather than a blocker.

## Downstream impact (feeds #19 / #20 / #22)

- **#19 (persistence):** store per-volume `lastEventId` (+ the `core` cursor) exactly as
  `events.json` does. **Additionally persist `nodeUid` on every indexed record** — a
  `node_deleted` event yields only `nodeUid`, so uid→path must be resolvable from our own
  index *after* the node is gone.
- **#20 (client capability):** the sidecar protocol replaces the trait sketch. Map
  `DriveEvent → RemoteDelta`; resolve `nodeUid`→path via the index (fall back to
  `ProtonDriveClient.getNode(uid)` for unknown uids). Resolve the auth open question first.
- **#22 (robustness):** `tree_refresh` / `fast_forward` = force a full BFS reconvergence and
  reset the cursor; `tree_remove` = volume gone; keep a periodic full scan as backstop.

## Status 2026-08-14 — the shipped cadence (what polls, what walks)

The loop this ADR motivated now runs at two cadences, and the earlier notes above (#22's "keep a
periodic full scan as backstop", and the original `EVENTS_POLL_INTERVAL` comment claiming
"full-tree snapshots stay on `scan_interval`") no longer describe what ships. Authoritative today:

- **`EVENTS_POLL_INTERVAL` (30s)** — the incremental (O(changes)) pass. Its select arm is armed only
  while `events_driven` is on **and** an event source exists. A degraded session (locked keyring,
  headless host, no CLI session) makes every pass a full-tree walk, so leaving the arm armed there
  meant an O(folders) BFS every 30s forever (#50); it is now gated off, and the cause is reported
  once — naming both intervals — through the same one-reason-per-cause latch
  (`note_event_scope_declined`) that reports a missing volume or cursor, so
  "event-driven detection unavailable" stays one message family rather than two overlapping lines.
- **`scan_interval`** — the *snapshot* cadence only when event-driven detection is not live (feature
  off, or degraded as above; that pass is also where the session is retried). With detection live a
  `scan_interval` tick is **just another incremental pass**, usually idle (#52). It is deliberately
  not made to force a snapshot: the periodic full-tree resync (`events_full_scan_every`) has been
  **opt-in and off by default** since PR #138, so forcing one here would reinstate exactly what that
  change removed.
- **Full-tree walks** therefore happen on: the first pass after boot (bootstrap, or the warm start's
  remote replay — ADR 0004), an event-stream fallback (no cursor / no volume / fetch error / server
  refresh / unresolvable node / a `Created` node its parent listing has not caught up with yet),
  `proton-sync resync` / `--full-walk`, the opt-in
  `events_full_scan_every` (in-run) and `warm_start_full_walk_every` (across restarts), and every
  pass while the session is unusable.

Corollary for **local** changes: with no periodic full walk, the idle fast-path is the only thing
standing between a dropped `notify` event and a change that is never re-derived. Directory events
are queued and a watcher error forces a local rescan (#51) for that reason.

## Prototype evidence (this spike, no code shipped)

No sidecar was run live (a real run needs an interactive Proton login, out of scope for the
spike). Reachability and schema were established from: the CLI's live event traffic
(endpoints, cadence), `events.json` (cursor shape), a cursor advance observed after a real
mutation (corroborating, not a controlled proof — see caveat above), and the SDK's published
`.d.ts` (API + delta schema). The auth **supply** surface was resolved by reading the SDK
deps/README and npm (above); the residual unknown is narrowed to candidate (1)'s feasibility
(keyring session reuse), which is #20's first task.

## Addendum 2026-07-21 — event detection is auth-only (#20)

Spiking #20's auth question turned up a finding that **narrows** (does not remove) the auth
cost above and reopens the implementation choice. Verified by reading the SDK source, not a
live run.

### Finding: deriving the delta needs no decryption

`dist/internal/events/apiService.js` imports only `../uids` and `./interface` — **no crypto,
no account, no `decrypt`.** `getVolumeEvents` builds every `DriveEvent` from **server-cleartext**
fields of the raw API response:

```js
// GET drive/v2/volumes/{volumeId}/events/{eventId}  →  { EventID, More, Refresh, Events[] }
events: result.Events.map((event) => ({
    type:          VOLUME_EVENT_TYPE_MAP[event.EventType],   // 0=del 1=create 2,3=update
    nodeUid:       makeNodeUid(volumeId, event.Link.LinkID),        // string composition, not crypto
    parentNodeUid: event.Link.ParentLinkID ? makeNodeUid(volumeId, event.Link.ParentLinkID) : undefined,
    isTrashed:     event.Link.IsTrashed,
    isShared:      event.Link.IsShared,
    eventId:       event.EventID,
    treeEventScopeId: volumeId,
}));
// getVolumeLatestEventId: GET drive/volumes/{volumeId}/events/latest → result.EventID
```

The node **name is not in the event at all** — events carry IDs, parent IDs, type, and the
trashed/shared flags, all cleartext.

### What this changes (and what it doesn't)

1. **Auth is narrowed, not removed.** The earlier claim — "needs an `account` with unlocked
   private keys + the PGP module" — is **false for detection**. The true requirement is a
   Proton **session**: `x-pm-uid`, `Authorization: Bearer <access>`, app version, and 401→
   refresh. That is a real reduction (session tokens ≪ unlocked key material + crypto), but
   **obtaining and refreshing a session is still the dominant cost.** Auth stays central.

2. **The saving is "bypass the SDK for detection," not "`iterateEvents` is cheap."** Public
   `iterateEvents` still hangs off a `ProtonDriveClient` whose constructor demands `account`
   + crypto, so going *through* the SDK saves nothing. The unlock is that the mapping above is
   trivial and now known verbatim, so a **thin auth-only client** (Rust-native, or a tiny
   sidecar) can call `drive/volumes/{id}/events/latest` + `drive/v2/volumes/{id}/events/{cursor}`
   and derive the delta itself — no SDK, no PGP.

3. **Detection ≠ full sync — the residual crypto.** Events give `nodeUid`, not names. For
   **known** nodes (delete / update / move) we resolve `nodeUid → local path` from our own
   index (#19). For a **newly-created** node, its name is still encrypted and must be decrypted
   (`ProtonDriveClient.getNode(uid)` or a targeted parent `filesystem list`). So crypto does
   not disappear — it shrinks from "decrypt the whole tree every scan" to "decrypt the names of
   nodes that just appeared."

### Consequence for #20

Because decryption dropped out of the **detection** path, detection no longer forces a
Node/SDK host. The host/language choice is now driven purely by **where a Proton session is
easiest to obtain and refresh**:

- **Thin auth sidecar (Node + keytar):** smallest new code; reuses the CLI's keyring session
  (candidate 1). Still the leading option.
- **Rust-native events client:** no sidecar at all for detection — but the engine must hold a
  Proton session in Rust (reuse the CLI's, or implement SRP + refresh). This is a **partial**
  revival of rejected option (c): its **PGP objection is gone**; its **Proton-auth objection
  stands** (SRP/refresh is still real work).
- **Full SDK sidecar:** no longer required *just to detect changes*. It re-enters only for
  new-node **name decryption** and for content up/download — features beyond detection.

Session-reuse feasibility (candidate 1) is unchanged in importance but re-scoped to **session
tokens, not key material**. Note: `secret-tool` did not surface the CLI's wallet entry here,
but that is a *tooling* limitation (guessed attributes vs KWallet) — **not** evidence that
reuse is infeasible. The real probe is a Node process using `keytar` with the CLI's own
service name; that is #20's first concrete task, now clearly scoped: **obtain a session →
one authenticated GET to `…/events/latest` → derive a delta from cleartext fields.**
