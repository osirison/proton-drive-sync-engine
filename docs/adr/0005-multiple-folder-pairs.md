# ADR 0005 — Multiple folder pairs in one daemon

- **Status:** Proposed (design only — no implementation ships with this ADR)
- **Date:** 2026-08-17
- **Issue:** #102 (E5 · Multiple folder pairs). This ADR is the "scoping pass" the maintainer's
  decision comment asked for; it does not close the issue.
- **Relations:** constrained by #23 (the `proton-drive` CLI is not concurrency-safe) and by
  `paths.rs`'s two-tier locking; reuses ADR 0001's per-volume event cursor and ADR 0004's warm
  start, both of which become per-pair; leaves ADR 0002's guard and ADR 0003's checkpoint commits
  untouched by construction.

## The shape, in six sentences

For a reader who needs to agree the shape rather than audit it:

1. **One process, one `proton-drive` client, one `CliGate`, N pairs** — forced by #23, and the
   corollary is that anything living on the client cannot be per-pair.
2. **Pairs are `[[pair]]` tables with a required `name`; a file with no `[[pair]]` is one implicit
   pair called `default`, forever** — so every existing config keeps working and nothing is
   rewritten on upgrade.
3. **The pair is already the unit of storage** (`<local_root>/.sync/sync_index.db` holds all eight
   tables), so multi-pair needs *no schema migration* — only 2N connections.
4. **The wire gains one field, `ControlRequest.pair`; one request always addresses exactly one
   pair**, omitted means the default pair, and "all" is a client-side loop rather than a reserved
   word.
5. **Passes are serialized by a due queue** over pairs, with explicit requests jumping ahead of
   timer-due ones; a pair waits out another pair's pass, which is inherent to #23 and not a defect.
6. **The GUI is the *expensive* part, not the cheap one** — the pair selector the issue remembers
   does not exist in design-v2, and adding one to the header fails the fidelity box gate on all 51
   frames until the prototype is re-drawn.

## Context

The engine syncs exactly one `(local_root, remote_root)` pair, and every module says "the" where a
multi-pair engine has to say "which": *the* local root, *the* index connection, *the* event cursor,
*the* first pass after boot, *the* status reply. #102 asks for many.

The trap this ADR exists to avoid is the one named in the decision comment: discovering at phase
four that phase one's config shape cannot express what the protocol needs. So the whole shape is
decided here — config, storage, wire, scheduler, UI, migration — before any of it is code.

### The constraint that closes off the obvious design

**One daemon. Not one daemon per pair.** Verified rather than assumed:

- `paths.rs::default_global_lock_path` is keyed on `$XDG_STATE_HOME`, deliberately *not* on the
  per-session runtime dir, and `Daemon::with_client_and_event_source` acquires it for the process
  lifetime. Its own doc says "only one `proton-syncd` may run per user account".
- The reason is #23: every daemon shells the same `proton-drive` binary, whose SQLite cache and
  session store are shared per user and lose to `SQLITE_BUSY` under concurrent use.
- `proton::CliGate` is the in-process half of the same rule, and it is **per client instance**
  (`ProtonDriveClient.gate: Arc<CliGate>`, shared by clones). #317 and #323 spent a week closing the
  last two non-daemon processes that spawned their own children.

So N pairs are N *scopes inside one process*, serialized through one gate. Everything below follows
from that.

**Corollary, and it is load-bearing: one client instance, shared by every pair.** N pairs holding N
`ProtonDriveClient`s would be N gates, which is no serialization at all — #23 reintroduced inside
one process, exactly the shape #99 created and `CliGate` was written to close. The `Daemon` already
holds `Arc<C>` and shares that instance with the IPC task; multi-pair extends the same rule. Its
direct consequence is a config rule, not a preference: everything that lives **on** the client —
`proton_cli`, `proton_timeout`, `proton_list_attempts` (i.e. `CommandPolicy`), the cancel flag, and
the `ProgressSink` — is **daemon-wide and cannot be made per-pair** without moving policy from the
client to the call. That is stated in the taxonomy below rather than discovered in phase 4.

## Decision

### 1. The pair is the unit of state; the process is the unit of the CLI

A `PairRuntime` owns everything a pass decides from, and the daemon owns everything the *process*
decides from. The split is not a judgement call — it is dictated by the corollary above plus "which
tree does this describe":

| Owned by `PairRuntime` (one per pair) | Owned by the daemon (one per process) |
| --- | --- |
| `local_root`, `remote_root`, `db_path`, `lockfile_path`, `scan_options` | `proton: Arc<C>` (one gate), `cancel_flag`, `socket_path`, `global_lock_path`, `log_filter` |
| `connection` (core) + the control plane's second connection | the control socket, its listener and `ControlPlane` |
| `pending_changes`, `authored_writes`, `force_local_rescan` | the `notify` watcher (one watcher, N watched roots) |
| `is_first_reconcile`, `warm_starts_since_full_walk`, `incremental_passes_since_full_scan` | `event_source` + `event_source_factory` (a **session**, not a volume — the volume is a per-call argument) |
| `last_sync`, `last_error`, `last_plan_summary`, `last_successful_sync_summary`, `status_history` | `auth: AuthState` (the Proton session; genuinely per-user) |
| `pending_deletions`, `last_failed_items`, `unsyncable`, `reported_unsyncable` | the degraded-session decline latch (per-user cause) |
| `pass_log`, `pass_history`, `pass_intent`, `apply_report`, the plan slot | `CommandPolicy` (`proton_timeout`, `proton_list_attempts`, `proton_cli`) |
| `index_totals` (+ its stale flag), the status/metrics sidecar paths, `download_batch_size` | `ipc_io_timeout`, `browse_gate_wait`, `events_poll_interval` (cadences and test seams) |
| the per-pair `paused`, `syncing`, `reconcile_seq`, `force_full_walk`, `reset_index` latches | `plan_pass` becomes per-pair too; only `auth` stays a single atomic |
| the volume/cursor decline latch (per-pair cause) | |

`event_scope_declined` is one latch today carrying both a per-user cause (`AUTH_DECLINE_REASON`)
and per-pair causes (no volume, no cursor). It splits along the same line: the session cause is
reported once per **process**, the volume/cursor causes once per **pair**. That is a fix to an
existing wart, not new machinery — "one reason per cause, said once, re-said on change" is
unchanged, it just gains the right scope for each cause.

**The `ProgressSink` stays single.** It lives on the shared client and reports into "the pair whose
pass is running", which is well-defined because passes are serialized (§5). A second sink per pair
would be a second thing to keep in step for no gain.

### 2. Config: `[[pair]]` tables, with the single-pair file remaining valid forever

```toml
# daemon-wide — the process, and the one shared CLI client
socket_path      = "/run/user/1000/proton-sync.sock"
log_level        = "info"
proton_cli       = "/usr/bin/proton-drive"
proton_timeout_secs  = 300
proton_list_attempts = 3

[[pair]]
name        = "documents"          # required, unique, the wire selector
local_root  = "~/Documents"
remote_root = "/Drive/Documents"
exclude     = ["*.tmp"]
deletion_policy = "ask_every_time"
scan_interval_secs = 300

[[pair]]
name        = "photos"
local_root  = "~/Pictures"
remote_root = "/Drive/Photos"
```

**Key taxonomy** — every existing key is classified, and the classification is part of the design
because it is what a later phase cannot renegotiate cheaply:

- **Per-pair:** `local_root`, `remote_root`, `db_path`, `lockfile_path`, `include`, `exclude`,
  `delete_approval` / `deletion_policy`, `conflict_suffix`, `scan_interval_secs`, `events_driven`,
  `events_full_scan_every`, `warm_start`, `warm_start_full_walk_every`,
  `warm_start_max_cursor_age_secs`, `download_batch_size`, `dry_run`.
- **Daemon-wide by nature:** `socket_path`, `log_level`.
- **Daemon-wide *because the client is shared*:** `proton_cli`, `proton_timeout_secs`,
  `proton_list_attempts`. Setting these per-pair would require either a second client (forbidden —
  one gate) or moving `CommandPolicy` from the client onto each call. Neither is worth it for a
  timeout; they are daemon-wide and this is the reason.
- **Never a key:** `global_lock_path` (fixed per user, so the single-instance guarantee holds
  regardless of flags).

`conflict_suffix` is per-pair but carries a caution: changing it orphans sidecars already on disk
(guard `changing_the_suffix_orphans_sidecars_written_under_the_old_one`). Per-pair does not make
that safer, it only makes it per-pair.

`dry_run` is per-pair as a *config value*, but the one-shot `proton-syncd --dry-run` preview needs
an answer of its own: it returns before `Daemon::new`, takes neither lock, and builds its own client
(#317), and `preview_plan(config)` takes today's fused single-pair `DaemonConfig`. Phase 1 changes
what that function is handed, so it must say which pair it previews: **the default pair unless
`--pair` names another**, one pair per invocation, for the same reason the wire addresses one pair
per request. Previewing every pair in one report would need the report to grow a pair dimension, and
a preview is a rehearsal of one tree.

**Rules, each with the precedent it copies:**

1. **A file that uses both spellings is a startup error naming both keys.** Top-level `local_root` /
   `remote_root` (or any other per-pair key at top level) *and* a `[[pair]]` table is refused,
   exactly as `deletion_policy` + `[delete_approval]` is refused today: one setting written two ways
   has no defensible precedence, and refusing is also what lets a round-trip writer know which
   spelling it may rewrite.
2. **A file with no `[[pair]]` is one implicit pair named `default`.** Nothing is rewritten, nothing
   is migrated, and every existing config keeps working untouched. This is the whole of the
   config-side migration (§7).
3. **`name` is required inside `[[pair]]`, and unique.** Matched byte-exactly on the wire; two names
   differing only in ASCII case are refused at startup, so a selector can never be ambiguous by
   accident (#298's rule applied one layer up). Charset `[A-Za-z0-9._-]{1,64}` so a name is always a
   safe CLI argument and never looks like a path.
4. **Roots may not collide or nest.** No two pairs may share a `local_root` or a `remote_root`, and
   neither may be an ancestor of another pair's. The local half has a concrete proof, not a
   principle: `index::is_sync_state_path` matches only the **first** component of a relative path,
   so a pair nested under another pair's root would have its `.sync` directory — index, WAL,
   metrics sidecar — scanned and uploaded as ordinary files by the outer pair. The remote half is
   the mirror (two pairs would plan opposing actions for one remote subtree). `db_path` and
   `lockfile_path` overrides must also be unique: `LockGuard::acquire` uses `try_lock_exclusive`,
   and `flock` treats two descriptors on one inode as independent even within one process, so a
   duplicated lockfile path fails at startup with "daemon already running" — true but incompre-
   hensible. The config check must therefore run *before* the locks are taken.
5. **Every one of these rules lives in `validate_file_config_text`.** That function is already the
   one place a file's post-parse rules live, and `gui-core`'s config writer calls it rather than
   re-deriving them (#135). A rule added anywhere else is a rule the GUI will write configs that
   violate.
6. **The order of `[[pair]]` tables is meaningful**: the first is the *default pair* (§4). It is
   meaningful for exactly one reason — wire back-compat — and §7 explains why that makes it
   principled rather than arbitrary.

**Where a *new* config key goes — the rule, not the list.** The taxonomy above is a snapshot; what
has to outlive it is the procedure, because keys are being added right now (#193's
`full_scan_schedule` is queued behind this ADR for exactly this reason). Three questions, answered
in order, and the first `yes` decides:

1. **Does it live on the shared `proton-drive` client?** (i.e. is it part of `CommandPolicy`, the
   executable path, or anything the one client instance is constructed with.) → **daemon-wide**,
   forced by §1's corollary, not chosen.
2. **Does it describe the process rather than a tree?** (the socket, logging, the locks.) →
   **daemon-wide.**
3. **Otherwise it describes *a tree* — what to sync, what to skip, how often, how a pass behaves,
   what a deletion needs.** → **per-pair**, and it belongs inside `[[pair]]`.

Applied to the two queued issues: **#193's `full_scan_schedule` is per-pair** — it replaces the
`scan_interval` field in Settings, and "sweep this folder weekly" is a statement about a folder.
**#217's ancestor summaries are not a config key at all** (see §3).

Two properties make this cheap rather than a migration, and they are the reason to state the rule
now rather than after #193 lands:

- **A per-pair key added at the top level *today* is already the implicit pair's key tomorrow.**
  Rule 2 says a file with no `[[pair]]` is one pair, so `full_scan_schedule = "weekly"` at top level
  needs no migration when this ADR lands — it is that pair's value, by definition. #193 can
  therefore ship before or after phase 1 without either one waiting for the other, provided it is
  *classified* per-pair from its first commit.
- **The classification is exhaustive and machine-checked, so a new key cannot be forgotten.** Phase
  1 adds a `KeyScope` classification covering every `FileConfig` field, with a test asserting each
  field appears in it exactly once — so adding a field without classifying it is a build failure,
  not a discovery in phase 4. That same classification is the *only* input to the GUI's
  "promote this file to `[[pair]]` form" rewrite (§7), so a later key is hosted by classifying it
  and nothing else.

**Flags.** `--local-root` / `--remote-root` and the other per-pair flags keep working and continue
to mean "the single pair", so `proton-syncd --local-root ~/x --remote-root /Drive/x` is unchanged.
Combining them with a multi-pair config file is refused by rule 1 — a flag cannot say *which* pair
it is amending, and inventing `--pair NAME --exclude ...` flag scoping is machinery this feature
does not need (the config file is the multi-pair interface, and the GUI writes it).

**Both directions of the version skew:**

- *Newer daemon, older config:* rule 2. Works untouched, forever.
- *Older daemon, newer config:* `FileConfig` is `#[serde(deny_unknown_fields)]`, so a `[[pair]]`
  file stops an older daemon at startup with `unknown field 'pair'`. That is the correct failure —
  loud, comprehensible, and non-destructive — and it is strictly better than the alternative of a
  daemon that starts and silently syncs one pair out of three. Accepted, and it is the one place in
  this design where "an older X must degrade" is answered with "an older X must refuse".

**One consequence of that deny makes phase 1 a hard prerequisite for everything else.**
`gui-core`'s `ConfigDoc::save` validates the **whole document** through
`config::validate_file_config_text` before writing (`gui/gui-core/src/config_io.rs`, guard
`validate_rejects_unknown_keys_so_the_daemon_cannot_be_bricked`). So today a config file containing
`[[pair]]` does not merely stop the daemon — it makes **every GUI save fail**, including saves of
entirely unrelated keys (log level, skip rules, deletion policy). There is no "hand-write a
multi-pair file and try it" path, and no "the GUI ships the selector first" path. `FileConfig` must
learn the key before anything else in this feature can move, which is exactly what phase 1 does and
why it is phase 1.

### 3. Index scoping: N databases, no schema change

The baseline already lives in a per-root `<local_root>/.sync/sync_index.db`
(`paths::default_state_db_path`), and **every** table the daemon decides from lives in that same
file: `file_index`, `remote_event_cursor`, `delete_approvals`, `withheld_deletions`,
`warm_start_state`, `unsyncable_items`, and the history pair `sync_passes`/`sync_events`. So the
storage question answers itself: **the pair is the unit of storage, and multi-pair needs no schema
migration at all.**

What multiplies is connections, not tables: the core's connection and the control plane's second
connection (the one three verbs use — `approve`/`deny`, `keep`, `activity`) both become one per
pair, i.e. `2N` handles. Both already set a busy timeout; nothing about their relationship changes.

Consequences worth stating:

- `reset_index_state` is per-pair by construction, and `reset-index` gains a pair selector rather
  than new semantics.
- `warm_start_state` is a single-row table, so per-pair is automatic; the same for the cursor rows.
- **Cross-pair aggregates become a merge, not a query.** "Today's bytes" and the pass history are
  per-database. The daemon already refreshes `PassHistory` once per pass into the published
  snapshot precisely so a 2s poll never touches SQLite; with N pairs it refreshes the finishing
  pair's and the reply carries per-pair history. A daemon-wide roll-up (all pairs, today) is a sum
  over N cached values, computed at publish time — never by summing `sync_events` rows, which the
  history invariant already forbids.
- A pair removed from the config keeps its `.sync` directory. Nothing deletes it, nothing scans it.
  Adding the pair back later warm-starts from its persisted cursor if it is fresh, and bootstraps
  if the cursor is past `warm_start_max_cursor_age` (default 7 days) — the existing gate answers
  the "pair was away for a while" case with no new machinery.

The rejected alternative is one database with a `pair_id` column on all eight tables: it would
require a real migration, add a predicate to every query, break the "state travels with the folder"
property of `.sync`, and buy only the cross-pair aggregate that §3 gets by summing N cached values.

**Where a *new* per-pair store goes — the rule.** Same reasoning as the config rule: the list of
eight tables is a snapshot, and #217 (per-file common-ancestor summaries for the conflict card) is
already queued behind this ADR. Three rules, and together they mean a new store is per-pair for
free and never needs a multi-pair migration:

1. **State derived from a pair's tree lives in that pair's `sync_index.db`, as a new table** — never
   in a new file beside it, never in a daemon-wide store, never in the GUI's `gui.toml`. That is
   what makes it per-pair automatically, what makes it travel with the folder, and what keeps the
   "one schema, N queries over it" property the history tables already have.
2. **Its key is the pair-relative path, stored as a BLOB when it is a path** — the
   `unsyncable_items` precedent, because a path that is not valid UTF-8 is exactly the case a TEXT
   key mangles (and #270 guarantees such paths are planned rather than dropped).
3. **`reset_index_state` truncates it if and only if the daemon *decides* from it.** That function
   is "forget everything the daemon has learned", and it is deliberately not "empty the database":
   the baseline, cursors, warm-start counter and approvals are truncated; the display-only
   `unsyncable_items` list is not. A new table must answer that question explicitly, and the answer
   is the same question as "would a stale row here change a sync decision".

Applied to **#217**: the ancestor summary is per-file state within one pair, so it is a new table in
that pair's database keyed by the BLOB relative path (rules 1 and 2), and it is **not** truncated by
`reset_index_state` (rule 3) — it is display evidence for a conflict card, and a sync decision never
reads it. Its lifecycle ("captured before a sidecar is written, dropped when the conflict resolves")
is then a per-pair question by construction, which is what makes it cheap here: nothing about it
becomes harder with N pairs, and nothing about it has to be revisited when phase 4 lands.

The one thing a new store must *not* do is introduce a cross-pair query, for the reason in the
bullet above: a daemon-wide number is a sum over N per-pair values computed at publish time, never a
query that joins pairs.

### 4. Control protocol: a selector on the request, one pair per request

`ControlRequest` gains one field:

```rust
/// Which pair this command addresses. `None` = the default pair (the first `[[pair]]`), which
/// is what a client predating multi-pair means by every verb it sends. `#[serde(default)]`.
#[serde(default)]
pub pair: Option<String>,
```

Six of `ControlRequest`'s seven fields are already `#[serde(default)]`, and `ControlRequest::new`
exists precisely so a caller need not name fields its command ignores — so the selector is a
straight application of the existing precedent, not a new mechanism.

**But the backward direction is a silent wrong-target, and it needs a client-side gate.** No IPC
type carries `deny_unknown_fields` and there is no protocol version field, so a **newer client's
`pair` field sent to an older daemon is dropped on the floor and the verb executes against that
daemon's one pair, with no signal.** For `status` that is harmless. For `reset-index`, `keep`,
`approve` and `apply` it is a destructive action on a pair the user did not name — strictly worse
than a break.

The fix is the `reset-index --yes` precedent exactly: **a gate the wire cannot express belongs in
the client.** A client given an explicit `--pair` (or a GUI with a pair selected) first reads
`status`; a reply carrying neither `pair` nor `pairs` is a daemon that predates multi-pair, and the
client **refuses** with "this daemon does not support multiple folder pairs; upgrade
`proton-syncd`". No `--pair`, no gate — the omitted case means the default pair, and an old
daemon's only pair *is* the default pair, so it is correct without asking.

**The gate leaves a residual race, and it is accepted rather than solved.** Between the client's
`status` and the destructive request the daemon can be restarted at an older version — and an
upgrade in progress is precisely when version skew exists. There is no fix available at this layer:
the wire cannot express "I mean pair X" to a daemon that does not know pairs, which is the whole
premise. The window is seconds, during a deliberate administrative action, and the alternative
(a protocol version handshake) is a second problem — see the alternatives table.

The rejected alternative was a new *verb* (which an old daemon rejects hard). It rejects too hard
and in the wrong shape: `ControlCommand` has no `#[serde(other)]`, so an unknown verb fails the line
parse, the daemon drops the connection without replying, and every client renders that as "cannot
reach the sync daemon" (`gui-core`'s `IpcError::Unreachable`, `proton-sync`'s "Is it running?").
A version mismatch that reads as "your daemon is down" is #103's bug, not its fix.

`ControlResponse` keeps every field it has today, **now describing the selected pair**, and gains:

- `pair: Option<String>` — which pair the single-pair fields describe.
- `pairs: Vec<PairSummary>` (`#[serde(default)]`) — name, roots, `paused`, `syncing`,
  `reconcile_seq`, `last_sync_epoch_secs`, `last_error`, pending-change and pending-deletion
  counts. Enough for a header, a selector, and a tray tooltip without N round trips.

An older client reads the top-level fields and sees the default pair, correctly and completely. A
newer client reads `pairs` and never needs the default at all.

**The reply must stay additive, and that is a hard constraint rather than a preference.** Nine of
`ControlResponse`'s fields carry no `#[serde(default)]` — `status`, `paused`, `pending_changes`,
`message`, `last_sync_epoch_secs`, `last_error`, `last_plan_summary`,
`last_successful_sync_summary`, `status_history` — and three separate tests feed a nine-field legacy
JSON literal and assert it still parses (`a_status_reply_carries_no_plan_and_an_older_daemons_reply_still_parses`,
`…parses_as_no_listing_and_no_verdict`, `control_response_without_activity_still_parses`). So the
tempting shape — moving the per-pair fields *into* the `pairs` array and leaving a thin envelope —
is closed off: it breaks every shipped client and three guards say so. The primary pair stays
flattened at the top level and `pairs` is added beside it.

**`reconcile_seq` is per-pair, and the top-level one is the selected pair's.** A single global
counter would be a correctness bug for the oldest client we support: `watch_syncnow` waits for
`reconcile_seq` to advance past its ack, and a global counter would be advanced by *another pair's*
pass, ending the wait early and reporting that pass's outcome. Per-pair, an old client's
`syncnow` → poll → verdict loop is exactly as correct as it is today, because it is talking about
one pair from beginning to end. `plan_seq`/`apply_seq` follow the same rule for the same reason.

**No reserved word, no fan-out on the wire.** "All pairs" is a **client-side loop** (`proton-sync
status --all` issues one request per pair, from the `pairs` list it just read), not a magic
selector value. A folder may legitimately be named `all`, a reserved sentinel collides on both the
wire and the UI's identity (the #140 lesson), and a wire-level fan-out would need partial-failure
semantics on every verb. One request, one pair, one answer.

**An unknown pair name resolves to nothing and acts on nothing, and a client tells by shape rather
than by sentence.** Answering the default pair for a mistyped selector is #246's shape: a
fall-through arm that reads like success. The reply's `pair` field is `Some(name)` **exactly when
the request's selector resolved**, so a client that sent `photos` and reads back anything else knows
structurally that it did not resolve — no prose to match, which is the bug #103 removed. `message`
carries the human text naming the configured pairs, and `reset-index`, `approve`/`deny`/`keep` and
`apply` do nothing at all in that case. (`ControlResponse` has no `ok` field today and does not gain
one; `status`/`message` plus the structural `pair` answer it, and `proton-sync` exits non-zero, the
way `list`/`plan`/`apply` already do for a non-success outcome.)

Verb-by-verb, the selector's meaning:

| Verb | Selector meaning |
| --- | --- |
| `status` | which pair the single-pair fields describe (`pairs` is always present) |
| `syncnow`, `resync`, `plan`, `apply`, `reset-index` | which pair's queue entry / latch |
| `pause`, `resume` | **per-pair.** "Pause everything" is the client's `--all` loop |
| `approve`, `deny`, `keep` | which pair's `delete_approvals` / `withheld_deletions`, and which pair's local root the path is relative to |
| `activity` | which pair's `sync_events`; the path argument is relative to that pair's local root |
| `list` | which pair's `remote_root` the *relative* frame resolves against. An **absolute** selector (#323) names a Drive location directly and is pair-independent — it already is today, and it is what `gui_core::folder_probe` uses to price a folder before any pair exists |
| `shutdown` | daemon-wide; the selector is ignored |

`RunningConfigInfo { local_root, remote_root, db_path }` is the wire's current assertion that there
is exactly one pair. It stays, describing the selected pair, and `PairSummary` carries the same
three fields per pair.

`proton-sync` gains a global `--pair NAME` and `--all`; with neither, it addresses the default pair
and prints exactly what it prints today. On a multi-pair setup the human-readable output names the
pair in its headline, because "everything is up to date" that silently means one of three folders is
the same lie as #246's.

Two client-side details that are easy to miss and expensive to find in phase 4:

- **`--json` is not uniform, so a pair marker on `ControlResponse` alone is invisible to six
  verbs.** `status`, `pause`, `resume`, `resync`, `reset-index`, `stop` and the approval verbs print
  the whole response; `history`, `activity`, `pending`, `list`, `plan` and `apply` print a
  *projection* (`response.history`, `response.file_history`, …) with the envelope discarded. The
  rule: **the projections do not change** — a script that asked for one pair knows which pair it
  asked for — and `--all` with `--json` prints a JSON array of `{"pair": name, …}` objects. That
  wrapper appears only under `--all`, a flag no existing script passes, so no output any script
  parses today changes shape.
- **`watch_syncnow` decides "scheduled" by string-matching the ack message** (`"sync scheduled"` or
  a message containing `"already in progress"`) and then waits for `reconcile_seq + 1` or `+ 2`.
  Per-pair scheduling must keep those exact substrings and keep "already in progress" meaning *this
  pair's* pass — a `syncnow` for pair B while pair A is mid-pass is `"sync scheduled"` with target
  `+ 1`, because pair B's own counter is what will move. Getting this wrong makes the client wait
  for a number that never arrives, which is the failure mode `a_counted_pass_is_running` and the
  `plan_pass` discriminator already exist to prevent.

### 5. Scheduling: a due queue over pairs, one pass at a time

Passes cannot overlap — one gate, one CLI. So the loop stops being "reconcile when a timer fires"
and becomes "pick the next pair to run":

1. Each pair has a `next_due` instant, from its own `scan_interval` (and, when it has a live event
   source and `events_driven`, the faster `EVENTS_POLL_INTERVAL`). The existing rule that a
   degraded session rides `scan_interval` rather than the fast poll (#50) is per-pair and unchanged.
2. Explicit requests (`syncnow`, `plan`, `apply`) are a FIFO queue that **jumps ahead** of
   timer-due pairs. This is what makes an interactive request feel interactive.
3. Among timer-due pairs, the scheduler picks by earliest `next_due`, breaking ties with a rotating
   cursor over config order, so a pair with a 30s cadence can never starve one with a 300s cadence.
4. A paused pair is skipped and its due time still advances (so resuming does not fire a backlog).

**The fairness cost is real and inherent, and this ADR does not hide it:** a pair waits out
whatever pass is running, and that can be a 30-minute bootstrap. There is no fix inside #23 — the
CLI is the serialization point. Two things soften it: `CliGate` is held for **one child**, not one
pass, so interactive verbs (`list`) still land in the gaps of a long walk; and the warm start (ADR
0004) means the common restart cost is O(changes) per pair rather than O(folders).

**Boot.** "First pass after boot always full-scans the local tree" is per-pair, and every pair must
get one. On startup every pair is seeded due-now in config order, and the first passes run
serialized through the same queue — rather than, as today, one reconcile before the loop starts.
That is strictly better: the control socket answers while pair 3 is still waiting, and a shutdown
interrupts the sequence at a pair boundary. `is_first_reconcile` clears per-pair and only on
success, unchanged.

**Watcher.** One `notify` watcher with N watched roots (`watcher.watch` is called per root), and
`handle_fs_event` routes an absolute path to the owning pair by longest-prefix match — unambiguous,
because rule 4 of §2 forbids nesting. A watcher **error** carries no path, and an inotify overflow
means events were lost *somewhere*, so it sets `force_local_rescan` on **every** pair. That is the
fail-safe reading of #51 and the only one available.

**Event scope.** The event source is one per process (it is a session; the volume is a per-call
argument). The cursor stays per-pair, in that pair's database, keyed by volume — so two pairs on
one volume keep two independent cursors over one stream. That is correct but not free; see §8.

### 6. GUI

**The issue's premise is wrong here, and this is the single biggest cost correction in this ADR.**
"The pair selector and header are already shaped for many" describes the **v1** UI: a 214px sidebar
with a `Folder pair` eyebrow label over one *static* card (`docs/design-v2/Current UI.dc.html`, the
old prototype kept for contrast — no `<select>`, no chevron, no second entry, no click target).
Design-v2 **deleted that sidebar entirely** and moved the pair to the footer or the seam labels
(`docs/design-v2/02-shell.md`: the header slot table is app mark · product name · spacer · status
chip · menu, and the doors "replace the old 214px left sidebar entirely"). The shipped header
(`gui/src/js/ui/chrome.js`) has the same five slots. **There is no pair selector to turn on.**

So the GUI is not the cheapest part. It splits three ways:

**Free — already multi-root.** The packaged file-manager emblem extensions
(`packaging/emblems/{nautilus,nemo}/…`) already walk *up* from a file to whichever ancestor holds
`.sync/sync_index.db`, cache a connection per database, and memoise only positive hits so a root
appearing later is picked up. Their own comment says "there may be more than one sync root". Nothing
to do. `gui-core` is nearly free too: `conflicts`, `index_read`, `sidecars`, `free_space`,
`folder_probe` and `plan` are already root-*parameterised* — every one takes the root or db path as
an argument and none reads a global.

**Mechanical — one pinch point.** `RuntimePaths` (`gui/src-tauri/src/config_path.rs`) holds one
`config_path`/`socket_path`/`db_path`/`local_root`/`remote_root`/`conflict_naming` plus single-slot
daemon-reported fallbacks, and every per-pair Tauri command resolves through
`effective_local_root()` / `effective_db_path()`. Make that struct pair-indexed and roughly fourteen
commands follow mechanically (`scan_conflicts`, `resolve_conflict`, `read_conflict_pair`,
`path_sync_status`, `search_files`, `skip_rule_usage`, `run_dry_run`, `apply_plan`, `open_*`,
`free_space`, the three approval verbs). Two cautions: `status_payload_remembering` caches
`response.config` into `RuntimePaths`, which is where a multi-pair reply first misbehaves; and
`effective_remote_root` was *deliberately* not written (`run_dry_run` must tell a configured root
from a daemon-reported one), so a pair-indexed refactor must preserve that distinction rather than
tidy it away. The selected pair persists in `gui.toml` (`gui_prefs.rs`), never in the daemon's
config — that file exists precisely because `deny_unknown_fields` bricks on GUI-local keys.

**Genuinely new, and the expensive part.**

- **`ConfigDoc` has no array-of-tables API.** It is surgical `toml_edit` access to top-level scalars,
  one string array, and exactly one hardcoded nested table (`delete_approval`). `[[pair]]` needs new
  surface — plus the kebab-case alias handling every key carries (`key_in_use`). And until phase 1
  lands, a `[[pair]]` file fails *every* save (§2), so this work cannot start earlier.
- **The fidelity gate will fail on any header change, and it is not the new node that fails it.**
  An unstamped node is simply not compared — but the header's `flex:1` spacer *is* stamped
  (`shell-spacer`, mapped in every full-window fixture) and pinned to a hard width
  (`frames/2a-settled.json` records `header/span[1]` at `w: 731.08`, compared at a 0.5px tolerance).
  Inserting anything into the 52px header row shrinks the flex absorber and fails the box gate on
  **every full-window frame**. Add the copy gate (every fixed string must appear verbatim in a drawn
  frame; exemptions are treated as defects), the hue gate (five settled frames must contain no
  saturated colour anywhere — so a folder swatch or an accent on the selector fails them), and the
  stale gate (the prototype and the fixtures must draw the same frame set, both directions).
  **Doing this honestly means editing `docs/design-v2/Drive Sync.dc.html` first**, re-running
  `fidelity:extract`, regenerating the frames, adding fixtures, `data-fid` mappings and copy-deck
  strings. The frames are the spec; the selector must be drawn before it can be built.
- **Tray and notifications name no folder at all today** — a grep for `local_root|remote_root|folder`
  across `tray.rs`, `tray_menu.rs`, `sni.rs`, `notify.rs` returns nothing. So the cost there is not
  code but *semantics*, and it is two questions this ADR leaves to the GUI phase: which state wins
  when pair A is syncing and pair B is in outage (one glyph, N pairs), and how the fixed
  `Pause syncing` row spends the per-pair `paused` that §4 already decides on (fan out as `--all`,
  or add a daemon-wide flag beside it — §8b) — bearing in mind `tray_menu.rs`'s own warning that "a
  stale menu dispatches the action its label promised, or none". Notification bodies already carry a
  root-*relative* path, which is ambiguous the moment there are two roots, so every such body needs
  the pair named.
- **The in-app emblem path disagrees with the packaged one.** `path_sync_status` opens
  `effective_db_path()` — one index — and takes a relative path with no discriminator, while the
  file-manager extensions resolve per file. Until it takes a root, the overlay and the in-app
  status can disagree under multiple pairs.

**What does not change:** `SyncActivity` stays singular and gains a pair name. Only one pass runs at
a time, so the live activity surface describes at most one pair; a per-pair activity array would
model a state that cannot exist. Likewise the plan/conflicts/deletions screens render one selected
pair, never two side by side.

### 7. Migration: nothing happens, and that is the design

First start of a multi-pair-capable daemon on an existing single-pair installation:

| Surface | What happens |
| --- | --- |
| Config file | Parsed by rule 2 as one implicit pair named `default`. **Not rewritten.** |
| Index (`<local_root>/.sync/sync_index.db`) | Opened as-is. No schema change, no migration, no version bump. |
| Per-root lockfile | Acquired as-is. |
| Global lock | Unchanged — still exactly one daemon per user. |
| Control socket | Same path, same protocol. Old `proton-sync` binaries keep working against it. |
| Sidecars (`<db>.status.json`, `<db>.metrics.json`) | Per-pair already (derived from `db_path`); the GUI's existing paths still resolve. |
| The user | Does nothing. |

The config is rewritten into `[[pair]]` form at exactly one moment: when the user **adds a second
pair** through the GUI. That rewrite must be the same kind of round trip `config_io` already does —
atomic, comment-preserving where it can be, and never triggered by an unrelated save. A
hand-written single-pair file is never rewritten by a daemon or a GUI that is only reading it, for
the same reason an existing `[delete_approval]` file is never rewritten into `deletion_policy`.

**The default pair is the first `[[pair]]`, and migration is what makes that principled.** The
default exists for one purpose: to answer clients that predate the `pair` field. Those clients also
predate multi-pair, so the only defensible answer for them is "the pair you have always been
talking to" — and after migration that pair is the first one. The footgun is real and is named
here: reordering the tables changes which pair an old client and an unqualified `proton-sync`
address. A `default = true` key would remove it at the cost of another key to validate and
round-trip; it is deliberately not in phase 1 and is listed as open (§8).

**Downgrade** (rolling the daemon back with a `[[pair]]` config) refuses to start with `unknown
field 'pair'`. Non-destructive, comprehensible, accepted.

## What this design does not solve

Named rather than pretended-away.

### 8a. Two pairs on one volume pay for each other's events — and one pair already pays today

This is the hardest technical fact in the feature and it is **pre-existing**, not introduced by
multi-pair. Chain, verified in code:

1. The event stream is **per volume**, not per folder (ADR 0001; the cursor is
   `remote_event_cursor.scope_id = volumeId`). `fetch_event_delta` fetches every change on the
   volume, with no scope filter before `reconstruct_remote`.
2. `TargetedResolver::resolve` places a created/updated node by listing its parent — but only if
   the parent has an index row (`path_for_proton_id`). A node whose parent is not indexed falls
   back to the root listing, and if it is not there either, returns `Err(...)`
   (`src/daemon.rs:6335`, "changed node … is not under any indexed parent or the remote root").
3. `reconstruct_remote` turns that `Err` into `Reconstruction::FallbackToSnapshot`, and the pass
   re-bootstraps with a full O(folders) walk.

So **any create or update anywhere else on the same volume costs the pair one full-tree walk.** For
one pair that is already true — touch anything in Drive outside `remote_root` and the next event
pass full-walks. For N pairs on one volume (the common case: a Proton user typically has one volume
for "My files") it is N-way: every pair's every change makes every other pair full-walk.

**It is a cost, not a correctness bug.** The fallback is safe by construction: the bootstrap
anchors its cursor *before* the walk (`capture_pre_snapshot_cursor`, #294/#303), so nothing is
missed and nothing is double-applied. What is lost is the entire point of event-driven detection.

A second instance of the same gap, entirely within one pair: a node created inside a folder created
in the **same delta** is also unplaceable, because the new folder has no index row until the pass
commits and the resolver cannot see the in-pass `uid_to_path` map that `reconstruct_remote` is
building. `mkdir` followed by copying files in is therefore a full walk today.

**Two candidates, and what decides between them:**

- **(a) Make the resolver placement-complete, then treat "unplaceable" as "outside this pair" and
  skip.** Two parts: pass `reconstruct_remote`'s in-pass `uid_to_path` to the resolver (fixes the
  same-delta-parent case outright, for one pair, cheaply), and — for pairs sharing a volume —
  consult *every* pair's index on that volume so a node placed by any pair is placed. There is a
  real argument that skipping then becomes **sound**: the cursor advances only in the final commit
  of a fully-successful pass, so a folder inside the pair is either already indexed (an earlier
  pass committed it) or still in this delta (an earlier pass held the cursor). The argument has one
  hole, and that hole is what decides this: a node this daemon just uploaded has an **empty**
  `proton_id` until a listing backfills it, so a folder the daemon itself created remotely may be
  unplaceable while genuinely inside the pair — and skipping there would advance the cursor past a
  create nothing re-derives, which is precisely #30.
- **(b) Leave it.** Accept a full walk per foreign-change burst, and rely on the walk being safe.

What would settle it: (i) whether the empty-`proton_id` window can be closed (does the commit of an
`Upload` / `CreateRemoteDirectory` record the composed id, or only a later listing?); (ii) a
measurement on a real account of how often foreign events actually arrive; (iii) whether the CLI
can place a node without a parent path — **it cannot**: every `proton-drive filesystem` verb is
path-addressed (`list`, `info`, `create-folder`, `upload`, `download`, `rename`, `copy`, `move`,
`trash`, `restore`, `delete`), so there is no uid→path call to lean on. That last one is settled,
and it is why (a) has to be built out of index lookups rather than a CLI question.

This belongs in its own ADR with its own live gate (the `tests/events_identity_live.rs` shape),
and it is phase 6 below — deliberately *after* multi-pair ships, because multi-pair is usable
without it and it is not usable without multi-pair to motivate it.

### 8b. Smaller open questions

- **What the tray's single pause toggle means.** The *wire and daemon state* are decided, not open:
  `paused` is per-pair (§1, §4), because everything else about a pair is. What is open is one layer
  up — the tray has one `Pause syncing` row, and pausing one of three folders from it would be a
  surprise. Two candidates: the row fans out as the client's `--all` loop (no new daemon state, but
  N requests and no atomic "everything is paused" reading), or a daemon-wide `paused` is added
  *beside* the per-pair one and a pair runs only when neither is set (one more flag, one more thing
  to publish, but an honest global). What decides it: whether design-v2 draws pause per-pair
  anywhere. Nothing below the tray depends on the answer, which is why it can be left to phase 5.
- **An explicit `default = true` key** instead of file order (§7). Cheap, removes the reordering
  footgun, costs a validation rule and a round-trip key. Deferred, not rejected.
- **A bound on the number of pairs.** Each pair costs two SQLite handles, a recursive inotify
  registration (`max_user_watches` is a real per-user kernel limit, and a large tree can consume
  tens of thousands of watches), and a share of one serialized CLI. No cap is proposed; a
  startup warning above some N is probably right, and what decides the number is a measurement of
  watch consumption on a real tree, not a guess here.
- **Whether a pair may be disabled without being deleted** (`enabled = false`). Trivially
  expressible; not designed here because nothing has asked for it.
- **Two pairs whose local roots sit on different filesystems** are already fine (the download
  staging dir is per-root), but a pair whose root is on a removable or network mount that is
  *absent* at boot is not designed for: today an unreadable `local_root` fails the pass, and with N
  pairs one such pair must not stop the others. The scheduler skipping a pair whose root is missing
  (and saying so once) is the obvious answer and is phase 4's smallest open detail.

## Phase plan

Six phases. Each leaves `main` green and shippable; the plan is ordered so the config shape — the
thing the decision comment worried about — is settled and *in the repo* before anything depends on
it.

**Phase 1 — Config shape and validation (small, and a hard prerequisite for everything else).**
Parse `[[pair]]`, implement rules 1–6 of §2 in `validate_file_config_text`, add the machine-checked
`KeyScope` classification, and resolve to a `Vec<PairConfig>` + a `DaemonConfig` holding only the
process-wide keys. **More than one pair is refused at startup** with "not yet supported". Nothing
else changes; the daemon still runs one pair, byte-identically. Closes: the shape question, the
version-skew story in both directions, and — because `ConfigDoc::save` validates the whole document
through the engine — the ability for the GUI to touch a multi-pair file at all. Nothing in phases
2–5 can start before this, and that is the point of putting it first.
Deliberately broken-but-unreachable: nothing — `N > 1` is refused, so there is no reachable
multi-pair path.

**Phase 2 — `PairRuntime` refactor (large, and entirely mechanical).** Move the ~25 per-pair fields
off `Daemon` into a `PairRuntime`, thread `&mut PairRuntime` through the reconcile family
(`reconcile_blocking`, `first_reconcile`, `bootstrap_reconcile`, `try_incremental_reconcile`,
`execute_plan_and_commit`, the plan/apply family), split `ControlShared` into a per-pair block plus
the per-user `auth`, and split the decline latch. The daemon holds `Vec<PairRuntime>` of length 1.
**Wire-identical rather than "no change by construction"** — `ControlShared`'s *internal* structure
does move, so what pins the output is the existing guards: the three legacy-JSON floor tests in
`src/ipc.rs` and `tests/ipc_cli.rs`, which drives the real binaries and asserts on JSON keys. If
those pass unchanged with one pair configured, the reply did not move. This is the phase where a
rushed job costs the most, because every invariant guard in `daemon.rs` runs through these
signatures. Closes: the "one connection, one index" assumption. Leaves broken: nothing.

**Phase 3 — Protocol selector (medium).** Add `ControlRequest.pair`, `ControlResponse.pair` and
`pairs`, make `reconcile_seq`/`plan_seq`/`apply_seq` per-pair, add `--pair`/`--all` to
`proton-sync` **and its client-side capability gate**, and make the unknown-pair refusal typed. With
one pair configured, every existing client and every existing test sees the same bytes it does
today. `tests/ipc_cli.rs`'s `wait_for_reconcile_seq` helper needs a per-pair form — it is the shape
every later multi-pair integration test is written against, so it is worth getting right here rather
than in phase 4. Closes: the wire. Leaves broken: nothing — `--all` over one pair is a loop of one.

**Phase 4 — Scheduler, and lift the `N > 1` refusal (the hard one).** The due queue, the multi-root
watcher and its routing, per-pair boot ordering, per-pair pause, the missing-root case. This is
where the fairness policy, the boot sequence and the shutdown behaviour are decided in code, and
where the tests are genuinely new rather than mechanical (two pairs, one gate, one queue: does a
`syncnow` on pair B jump a timer-due pair A? does a 30-minute pair A bootstrap starve pair B's
watcher-driven pass? does a shutdown mid-pair-2 leave pair 1 committed and pair 3 untouched?).
Expect this phase to be as large as phase 2 and riskier. Closes: the feature, headlessly.

**Phase 5 — GUI (large, and larger than the issue assumes).** Splits into three, and they are worth
tracking separately because only the first is mechanical: (5a) pair-index `RuntimePaths` and the
~14 commands that follow, plus the selected pair in `gui.toml` — mechanical; (5b) `ConfigDoc`'s
array-of-tables API and the "promote to `[[pair]]` form" rewrite driven by phase 1's `KeyScope` —
new surface, no analogue in the file today; (5c) **the selector itself, which starts in
`docs/design-v2/Drive Sync.dc.html`, not in `chrome.js`** — re-draw, `fidelity:extract`, regenerate
51 frames, fixtures, `data-fid` mappings, copy deck, and re-check the five settled frames for hue.
Plus the tray/notification aggregation semantics (§6), which are two decisions and little code.
Closes: the feature for users.

**Phase 6 — Shared-volume event scope (its own ADR).** §8a. Independent of everything above and
worth doing on its own merits, since one pair already pays the cost. Not scheduled here.

An honest ordering note: phase 1 and phase 3 could each ship in a week. **Phases 2, 4 and 5c are the
three that deserve their own review cycles** — 2 because every invariant guard in `daemon.rs` runs
through the signatures it moves, 4 because its tests are genuinely new rather than mechanical, and
5c because the fidelity gate makes a header change a 51-frame change. Phase 4 should not start until
phase 2 has been merged and lived on `main` long enough to shake out the guards. Phases 5a and 5b
can run in parallel with 3 and 4 once 1 is in.

## Alternatives considered

| Option | Verdict | Why |
| --- | --- | --- |
| One daemon per pair | **Closed off** | The user-global lock (#23): every daemon shells one non-concurrency-safe CLI. This is the constraint, not a preference. |
| One daemon, N clients | **Rejected** | N clients is N `CliGate`s, i.e. #23 reintroduced in-process. One client, shared. |
| One database with a `pair_id` column | Rejected | A real migration on eight tables and a predicate on every query, to buy an aggregate that is a sum of N cached values. The per-root `.sync` layout already scopes storage. |
| A `pairs = ["name"]` selector on the wire (fan-out) | Rejected | Partial-failure semantics on every verb, and a reserved-word collision (#140). One request, one pair; "all" is a client loop. |
| A separate socket per pair | Rejected | The socket must stay locatable without knowing a root (`paths.rs`), `sun_path` is short, and it would multiply the control plane for a question a selector field answers. |
| Omitted selector means "all pairs" | Rejected | Defensible for `syncnow`, alarming for `reset-index`, and it makes the omitted case mean two different things depending on the verb. Omitted always means the default pair. |
| Derive the pair name from `local_root`'s basename | Rejected | Two folders named `docs` under different parents collide, and the collision surfaces as an ambiguous wire selector rather than a config error. `name` is required. |
| Keep a single global `reconcile_seq` | Rejected | `watch_syncnow` would have its wait satisfied by another pair's pass and report that pass's outcome — a correctness bug for the oldest client we support. |
| A new pair-aware *verb* instead of a field on the request | Rejected | It does fail hard on an old daemon — but in the wrong shape: `ControlCommand` has no `#[serde(other)]`, so the line parse fails, the connection is dropped with no reply, and every client renders it as "the daemon is unreachable". A version mismatch that reads as an outage is #103's bug. The field plus a client-side capability gate (§4) fails *legibly*. |
| A protocol version field | Not needed for this | It would be the general answer, but the wire has never had one and this feature needs exactly one question answered ("does this daemon know about pairs"), which `status` already answers by carrying `pairs` or not. Adding a version now would be designing for a second problem while solving the first. |

## Invariants preserved, and what each costs under N pairs

Every invariant in `CLAUDE.md` survives; several become *per-pair* statements, and that is the whole
of the change. The ones worth naming:

- **Commit-after-side-effects, checkpointed.** Untouched: checkpoints are per-pass, and passes are
  serialized. The cursor rule ("advances only in the final commit of a fully-successful pass") is
  per-pair and per-volume, which is exactly what it already is.
- **First pass after boot full-scans the local tree.** Per-pair, and every pair must get one before
  it may take the idle fast-path. This is what makes boot O(N) passes; the warm start is what makes
  each of them cheap.
- **Selective-sync filters apply everywhere.** Per-pair `ScanOptions`, unchanged. The one new
  requirement is that the *routing* of a watcher event to a pair happens before the pair's filter
  runs, or an event under pair A's root would be tested against pair B's globs.
- **A wire path is a rendering, never a selector.** Extends to the pair name: names are matched
  byte-exactly, and a name that resolves to no pair authorises nothing.
- **Delete-approval guard, partial-pass state, vanished-node skip, history-behind-side-effects.**
  All per-pair by construction once storage is per-pair; none of them acquires a cross-pair
  dimension.
- **`scan_interval` is not a snapshot cadence in events mode; a degraded session is.** Per-pair
  cadence, per-*process* degraded session. This is the one invariant whose two halves land on
  different sides of the split, which is why §1 splits the decline latch explicitly.

## Findings that refine the issue and the decision comment

1. **The decision comment's "one event cursor per volume today; multiple pairs may share a volume
   or not" understates it.** Sharing a volume is not a scheduling nuisance, it is the §8a fallback
   problem — and that problem *already fires for a single pair* whenever anything changes elsewhere
   in Drive. Multi-pair does not create it; it multiplies it.
2. **Index scoping is the cheap part, not a hard part.** The comment lists "the control plane's
   second connection, the approval writes, `reset_index_state`, and the history tables" as things
   "written as *the* database". They are — but they are all written as *one* database that is
   already per-root, so they multiply as connections and need no schema work. The expensive part of
   the storage story is `ControlShared`, not SQLite.
3. **"The UI selector already exists in shape" does not hold, and it is the largest cost
   correction here.** The selector the issue remembers is v1's static `Folder pair` card in a
   sidebar design-v2 deleted; the shipped and designed headers both have five slots and none of them
   is a folder. The GUI is not the cheapest part of this feature — the fidelity gate makes a header
   change a 51-frame change, and `ConfigDoc` has no array-of-tables API. What *is* free is the part
   nobody counted: the packaged file-manager emblem extensions are already multi-root by design.
4. **A `pair` field on the request degrades *silently*, not safely.** No IPC type carries
   `deny_unknown_fields` and there is no version field, so an older daemon ignores the selector and
   acts on its own pair. For `reset-index`/`keep`/`approve`/`apply` that is a destructive
   wrong-target, which is why §4 puts a capability gate in the client rather than trusting the
   wire's forgiveness.
5. **`deny_unknown_fields` makes the older-daemon *config* case a refusal, not a degrade.**
   Everywhere else in this codebase the back-compat rule is "an older X degrades"; on the config
   file it cannot, and refusing is the better answer. Worth naming because it is the one asymmetry —
   and because the same deny is what makes a `[[pair]]` file fail every GUI save until phase 1
   lands.
