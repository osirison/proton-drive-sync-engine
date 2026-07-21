# ADR 0001 — Reach Proton Drive change events via an SDK sidecar

- **Status:** Proposed
- **Date:** 2026-07-21
- **Issue:** #18 (spike + ADR) · Epic #16 · Related: #23 (CLI not concurrency-safe)
- **Supersedes framing of:** #17 (parallel `filesystem list` BFS — deferred)

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

## Prototype evidence (this spike, no code shipped)

No sidecar was run live (a real run needs an interactive Proton login, out of scope for the
spike). Reachability and schema were established from: the CLI's live event traffic
(endpoints, cadence), `events.json` (cursor shape), a cursor advance observed after a real
mutation (corroborating, not a controlled proof — see caveat above), and the SDK's published
`.d.ts` (API + delta schema). The auth **supply** surface was resolved by reading the SDK
deps/README and npm (above); the residual unknown is narrowed to candidate (1)'s feasibility
(keyring session reuse), which is #20's first task.
