# ADR 0005 — Multiple folder pairs in one daemon

- **Status:** Implemented through phase 4 — a config may declare any number of pairs and one daemon
  runs them all (the lift, phase 4c). Phase 5 (the GUI) and phase 6 (the shared-volume event scope)
  are not built. Written as a design (2026-08-17); the phases below carry their own "Shipped, with
  departures" notes.
- **Date:** 2026-08-17
- **Issue:** #102 (E5 · Multiple folder pairs). This ADR is the "scoping pass" the maintainer's
  decision comment asked for; it does not close the issue.
- **Relations:** constrained by #23 (the `proton-drive` CLI is not concurrency-safe) and by
  `paths.rs`'s two-tier locking; reuses ADR 0001's per-volume event cursor and ADR 0004's warm
  start, both of which become per-pair; leaves ADR 0002's guard and ADR 0003's checkpoint commits
  untouched by construction.

## In plain terms

*Added after review: the maintainer's verdict on the first draft was "very long and convoluted", and
a design document nobody can decide from is one that gets ignored. This section is the whole design
for a reader deciding whether to build it. Everything below it is the audit trail.*

**What we are building.** Today the app syncs one local folder to one Proton folder. This lets it
sync several — `Documents → /Docs` and `Photos → /Pictures` — each independently.

**The one constraint that shapes everything.** We cannot run one copy of the daemon per folder. The
`proton-drive` command-line tool corrupts its own database if two copies run at once (#23). So:
**one daemon, all folders, one at a time.** The practical consequence is that **folders take turns**
— if two are due at the same moment, one waits. That is forced by the tool, not chosen.

**How it works, in six pieces:**

1. **Declaring folders.** The settings file gains a list; each entry is a local folder, a Proton
   folder, and a name. A file with *no* list is treated as one folder called `default`, so every
   existing config keeps working untouched.
2. **Folders cannot nest inside each other.** Not tidiness: the "ignore my own bookkeeping folder"
   check only looks at the top path component, so a folder inside another folder would have its
   private `.sync` directory uploaded to Proton as if it were the user's files.
3. **The database needs no conversion.** Each folder *already* keeps its own database in a hidden
   `.sync` directory beside it, holding all eight tables. A second folder simply gets its own. This
   was expected to be the hard part and is not.
4. **Commands say which folder.** Every command — sync now, pause, approve a deletion, preview a
   plan — gains a "which folder" field. Omitted means the default folder. There is deliberately **no
   "all" keyword**; the client repeats the command per folder, because a magic word that means
   something different in two places has bitten this project before (#140).
5. **Taking turns has an order.** Folders due for a pass form a queue; anything the user asks for by
   hand jumps ahead of anything on a timer.
6. **Upgrading does nothing.** No conversion, no version bump, no user action. The file is only
   rewritten into list form at one moment: when the user adds a second folder.

**Three problems this design work uncovered, none of which the issue predicted:**

- **The app window has no room for a folder switcher.** The old design had one in a sidebar;
  design-v2 deleted it. Layouts are checked pixel-by-pixel, and the header's flexible spacer is
  pinned to an exact width on 22 frames — so inserting anything there fails the gate on all of them.
  **The GUI is the expensive part, not the cheap one.**
- **An old daemon with a new client is dangerous.** The "which folder" field would be *silently
  ignored* by a daemon that predates it, which would then act on its own folder. For `reset-index`,
  `keep` or `approve` that is a destructive wrong target, so the client checks capability before
  sending one.
- **An existing cost gets multiplied.** Today a change *anywhere* in the user's Drive — even outside
  the synced folder — forces a full-tree walk. With N folders that happens N times. Not a
  correctness bug; worth knowing before multiplying it.

**Build order** (engine first, per the maintainer's decision on 2026-08-17): phases 1–4 make
multiple folders work fully from the command line; the GUI selector (phase 5c) waits for re-drawn
frames; the multiplied-walk cost (phase 6) gets its own ADR.

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
   does not exist in design-v2, and adding one to the header fails the fidelity box gate on the 22
   frames that pin the header's spacer (20 `window` + 2 `dialog`, measured) until the prototype is
   re-drawn.

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
   safe CLI argument and never looks like a path — **plus the two refusals that charset cannot
   express**, without which the justification is a guarantee the code does not give (#339, found in
   phase 1's implementation): `.` and `..` are spelled entirely within it and *are* path components,
   and `-h` / `--pair` are spelled entirely within it and *are* option syntax. So a name may not be
   `.` or `..`, and may not start with `-`. The length bound is checked *after* the charset, because
   `str::len` is bytes and a 40-character accented name is 80 of them — reported as "longer than 64
   characters", which names the wrong problem. Once the charset holds, bytes and characters agree.
   **`default` is reserved for the first table.** It is the name of the pair a request that names no
   pair addresses (rule 6 / §7), so a *later* table called `default` gives one selector two answers
   — the same sentinel collision §4 refuses for `all`, reintroduced for `default` and caught in the
   phase-1 review. It is not refused outright: the first pair may legitimately be called that (it is
   what the implicit pair is already named, and what §7's rewrite would write for a pre-existing
   single-pair file).
4. **Roots may not collide or nest.** No two pairs may share a `local_root` or a `remote_root`, and
   neither may be an ancestor of another pair's. The local half has a concrete proof, not a
   principle: `index::is_sync_state_path` matches only the **first** component of a relative path,
   so a pair nested under another pair's root would have its `.sync` directory — index, WAL,
   metrics sidecar — scanned and uploaded as ordinary files by the outer pair. The remote half is
   the mirror (two pairs would plan opposing actions for one remote subtree). `db_path` and
   `lockfile_path` overrides must also be unique: `LockGuard::acquire` uses `try_lock_exclusive`,
   and `flock` treats two descriptors on one inode as independent even within one process, so a
   duplicated lockfile path fails at startup with "daemon already running" — true but incompre-
   hensible. The config check must therefore run *before* the locks are taken. **The local half's
   proof reaches state paths too**, which phase 1 missed by writing the rule root-vs-root (#339): a
   `db_path` explicitly placed inside *another* pair's `local_root` produces the same upload with no
   root nested anywhere, because `scan_options_from_config` is handed only that pair's own
   `db_path`. So a pair's state paths may not be another pair's, nor sit inside another pair's
   `local_root`.

   **Which of these are structural, and which a flag can mask.** Every rule above is about *two*
   pairs, and a flag can name no pair — that is what makes them structural and what lets them run
   before the merge. The same-pair question they were originally written with (one file named as
   both a pair's index and its lockfile) is **not**: `--db-path` and `--lockfile-path` replace both
   values, so it belongs with the other flag-maskable rules — checked on the file's own values for a
   reader that has no flags, and on the merged values for the daemon. Phase 1 had it in the
   structural layer, where it refused a `[[pair]]` file over two values its flags replaced while the
   top-level spelling was never checked at all. The corollary is the layer's rule: **nothing here
   may refuse a value a flag replaces**, which is also why the comparison expands `~` best-effort
   and compares an unexpandable one verbatim instead of erroring.
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
  field appears in it exactly once, rather than leaving it a discovery in phase 4. That same
  classification is the *only* input to the GUI's "promote this file to `[[pair]]` form" rewrite
  (§7), so a later key is hosted by classifying it and nothing else.

  **Correction (#339): it is not a "build failure", and the two halves fail differently.** The
  compiler's half is the exhaustive struct literal in the fixture — it forces a new field to be
  *mentioned*, nothing more. The test's half is what refuses a field with no variant, and in phase 1
  as merged it did not: the key set was read through `toml::Value`, TOML has no null, so a field
  left `None` was omitted from **both** sides of the comparison. `None` — the natural value for a
  fresh `Option` — therefore satisfied the compiler and disappeared from the test, and a real,
  parseable, unclassified key passed every guard, including rule 1's top-level check. The key set is
  read through JSON now, and a test pins the property itself: an all-`None` struct must have the
  same keys as a fully populated one. Worth stating exactly, because #193's `full_scan_schedule` is
  the next key in the queue and this is the guard it lands on.

  That property is **narrowed, not closed**, and overstating it in the new direction would be the
  same defect with the opposite sign: `#[serde(default, skip_serializing_if = "Option::is_none")]`
  on a new field reopens the hole exactly as `None` did (measured — the whole suite is green under
  that poison). The guard covers the value a field is *left at*; it cannot cover a serde attribute
  that removes the field from the serialization on purpose.

**Flags, and the constraint the structural layer rests on.** `--local-root` / `--remote-root` and
the other per-pair flags keep working and continue to mean "the single pair", so
`proton-syncd --local-root ~/x --remote-root /Drive/x` is unchanged.

**The pair-set rules are unmaskable only while a flag cannot name a pair.** That is what lets
`resolve_pairs` run before the merge and still never refuse a value the daemon will not use: a flag
amends *the* pair, so it can change no answer about how two pairs relate. A phase that adds
`--pair NAME --local-root X` breaks that premise and reinstates #339 one layer up, inside the very
function this rule declares safe — every cross-pair comparison would then be running on values a
flag replaces. Whichever phase wants per-pair flag scoping owns moving those comparisons after the
merge, or proving they cannot be flag-addressed; it is not a free addition.
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
status --all-pairs` issues one request per pair, from the `pairs` list it just read), not a magic
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
| `pause`, `resume` | **per-pair.** "Pause everything" is the client's `--all-pairs` loop |
| `approve`, `deny`, `keep` | which pair's `delete_approvals` / `withheld_deletions`, and which pair's local root the path is relative to |
| `activity` | which pair's `sync_events`; the path argument is relative to that pair's local root |
| `list` | which pair's `remote_root` the *relative* frame resolves against. An **absolute** selector (#323) names a Drive location directly and is pair-independent — it already is today, and it is what `gui_core::folder_probe` uses to price a folder before any pair exists |
| `shutdown` | daemon-wide; the selector is ignored |

`RunningConfigInfo { local_root, remote_root, db_path }` is the wire's current assertion that there
is exactly one pair. It stays, describing the selected pair, and `PairSummary` carries the same
three fields per pair.

`proton-sync` gains a global `--pair NAME` and `--all-pairs`; with neither, it addresses the default pair
and prints exactly what it prints today. On a multi-pair setup the human-readable output names the
pair in its headline, because "everything is up to date" that silently means one of three folders is
the same lie as #246's.

Two client-side details that are easy to miss and expensive to find in phase 4:

- **`--json` is not uniform, so a pair marker on `ControlResponse` alone is invisible to six
  verbs.** `status`, `pause`, `resume`, `resync`, `reset-index`, `stop` and the approval verbs print
  the whole response; `history`, `activity`, `pending`, `list`, `plan` and `apply` print a
  *projection* (`response.history`, `response.file_history`, …) with the envelope discarded. The
  rule: **the projections do not change** — a script that asked for one pair knows which pair it
  asked for — and `--all-pairs` with `--json` prints a JSON array of `{"pair": name, …}` objects.
  That wrapper appears only under `--all-pairs`, a flag no existing script passes, so no output
  any script parses today changes shape.
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
the router (`Daemon::route_event`) hands an absolute path to the owning pair by longest-prefix match — unambiguous,
because rule 4 of §2 forbids nesting. A watcher **error** carries no path, and an `Ok` event flagged
`Rescan` (how notify reports an inotify overflow, #423) carries none on Linux (the macOS FSEvents
backend may attach one). Either means events were lost *somewhere*, so it sets
`force_local_rescan` on **every** pair. That is the
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
  **every frame that pins that spacer — 22 of the 51, measured: all 20 `window` frames and 2
  `dialog` frames, at five distinct widths (563.88, 696.08, 709.28, 731.08, 763.88), each compared
  exactly**. Add the copy gate (every fixed string must appear verbatim in a drawn
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
  `Pause syncing` row spends the per-pair `paused` that §4 already decides on (fan out as `--all-pairs`,
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

The *second* footgun in the same sentence was not named here and was found in phase 1's review
(#339): the word `default` is simultaneously this pair's **name** and the wire's **omitted
selector**, so `[[pair]] name = "photos"` followed by `[[pair]] name = "default"` makes an omitted
selector address `photos` while `--pair default` addresses the other one. §2 rule 3 now reserves the
name for the first table, which is the only reading under which the two surfaces agree — and which
keeps this rewrite free to name the pre-existing pair `default`, since it is the first one.

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
   back to the root listing, and if it is not there either, returns `Err(...)` (`src/daemon.rs`,
   grep for "is not under any indexed parent or the remote root"; this cited `:6335` and the line
   has moved).
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
  surprise. Two candidates: the row fans out as the client's `--all-pairs` loop (no new daemon state, but
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

> **Shipped, with two departures from the paragraph above — recorded here because both change what
> phase 3/4 will find.** (1) The pair is threaded as the *receiver* of a borrowed view,
> `PairPass<'a, C>` = `&mut PairRuntime` + the shared process pieces, rather than as a `&mut
> PairRuntime` parameter on each signature: the literal form does not compile, because
> `self.method(&mut self.pairs[0])` borrows `self` twice, and a `&self` receiver overlaps
> `self.pairs` just as badly. Destructuring the daemon's fields once, in `Daemon::pass()`, is the
> split the borrow checker proves disjoint — and it makes "a pass cannot see another pair" a property
> of scope rather than a rule forty methods remember. `pass()` is the single seat of the choice
> phase 4's due queue replaces. (2) The *runtime* holds `ProcessConfig` + `PairConfig`
> (`DaemonConfig::into_parts`, consuming, so no second copy of `local_root` exists), but the resolved
> **input** `DaemonConfig` stays fused and flat. Making the input a `Vec<PairConfig>` is the same
> change as lifting `refuse_unsupported_pair_count` — one flag cannot say which pair it amends —
> so it belongs to phase 4, and phase 1's comment promising it here has been corrected.

**Phase 3 — Protocol selector (medium).** Add `ControlRequest.pair`, `ControlResponse.pair` and
`pairs`, make `reconcile_seq`/`plan_seq`/`apply_seq` per-pair, add `--pair`/`--all` to
`proton-sync` **and its client-side capability gate**, and make the unknown-pair refusal typed. With
one pair configured, every existing client and every existing test sees the same bytes it does
today. `tests/ipc_cli.rs`'s `wait_for_reconcile_seq` helper needs a per-pair form — it is the shape
every later multi-pair integration test is written against, so it is worth getting right here rather
than in phase 4. Closes: the wire. Leaves broken: nothing — `--all` over one pair is a loop of one.

> **Shipped, with departures — recorded here because both phase 4 and phase 5 (the GUI) will find
> them.**
>
> (1) **The client flag is `--all-pairs`, not `--all`.** `approve`/`deny`/`keep` already have a
> per-command `--all` (every currently pending deletion, on the one pair they address); a *global*
> `--all` with the same long name panics `clap`'s `Command::build` in debug the moment a subcommand
> defines its own arg of that name. The paragraph above named `--all` because the ADR was written
> before this collision was found, and keeps that name as the historical record the departure
> pattern is for — the correction lives here, not in the prose above it. §4, §6 and §8b described
> the same global flag before this collision was found and are corrected in place; `--all-pairs` is
> the name everywhere else in this document, including in a future GUI's copy deck.
>
> (2) **`ControlShared.pair: PairShared` did not become `pairs: Vec<PairShared>` with the ~30
> methods threaded a `pair: &PairShared` parameter — most of them moved onto `impl PairShared`
> instead**, unchanged in body but for `self.pair.FIELD` becoming `self.FIELD`. Only the handful
> that also read `ControlShared::auth` (`response`, `metrics`, `response_with_sampled_activity`)
> stay on `ControlShared` and take `pair: &PairShared` explicitly. This is "one mechanism, not
> thirty ad-hoc lookups" read literally: a method that only ever touches one pair's own state has
> no business asking `ControlShared` which pair that is. `ControlShared::pair()` (index 0) is the
> one daemon-core-wide default; `resolve_pair_index` is the one wire-selector resolver, used by
> `handle_control_connection` alone.
>
> (3) **The progress-sink routing question (not in the ADR) resolved to "the pair whose
> `PairShared.syncing` is true," with no new state.** Passes are serialized (§5), so at most one
> pair is ever syncing; `ControlShared::active_pair` finds it, and `SharedProgressSink` drops a
> callback when none is active rather than guessing. The "browse must not pollute another pair's
> activity" risk turned out to be **structurally absent today**, not merely
> guarded against: `list_one_directory` (what `list`'s `browse_directory` calls) reports through
> neither `ProgressSink` method — verified by reading `proton.rs`, not assumed — so a browse cannot
> reach `active_pair` at all. The routing is index-based anyway, on the day either changes.
>
> (4) **`--all-pairs` does not watch each pair's pass to completion; it schedules and reports each
> pair's immediate reply.** `syncnow`/`apply`/`plan`'s watch loops (spinner, poll-until-sealed) are
> written for one command watching one pass; N interleaved on one terminal is unreadable, and a
> JSON array element cannot also carry a live progress stream. `--pair NAME` still watches to
> completion exactly as an unselected invocation does — `--all-pairs` is the fan-out-and-report
> tool, `--pair` is the one-pair-to-completion tool. Recorded as a scope decision, not a gap: phase
> 4 is free to revisit it once a real second pair exists to watch.
>
> (5) **`--all-pairs --json`'s per-pair wrapper is `{"pair": name, "result": <the same value a
> single-pair `--json` invocation prints for that verb>}`, uniformly** — not "a name plus the
> verb's fields spread at the top level," which the ADR's `{"pair": name, …}` sketch left open and
> which cannot be made uniform across verbs whose non-`--all-pairs` JSON output is sometimes an
> object (`status`), sometimes an array (`pending`) and sometimes absent (`pause`, a plain-message
> reply printed only under `--json` as the whole envelope). Nesting under `result` is one shape for
> every verb — `stop` excepted, see (7).
>
> (6) **An unresolved `--pair` now fails every verb's exit code, not just the ones that were
> already typed non-zero.** The wire's `pair: None` was always the structural signal (§4's
> `unknown pair` rule), but `status`/`pause`/… previously exited 0 unconditionally on the CLI side.
> `run_for_pair` checks `response.pair.is_none()` once, right after the request lands, before any
> per-verb rendering runs — the same "exit non-zero on a non-success outcome" rule `list`/`plan`/
> `apply` already followed, extended to every verb rather than duplicated per arm.
>
> (7) **`stop` does not print (5)'s `[{"pair", "result"}, …]` array under `--all-pairs --json` —
> it prints the bare `ControlResponse` envelope, once.** `fca4ab2` made `stop` daemon-wide (§4's
> verb table), so `run_all_pairs` special-cases it to one `run_for_pair(cli, socket_path, style,
> None)` call before the per-pair loop that builds (5)'s array even starts — the same early return
> that stops it sending N shutdown requests. A script branching on `--all-pairs --json stop`'s
> shape by verb, not by "every verb under `--all-pairs --json` is an array," gets this right;
> nothing else in the CLI has a second daemon-wide verb to generalize the exception to yet.

**Phase 4 — Scheduler, and lift the `N > 1` refusal (the hard one).** The due queue, the multi-root
watcher and its routing, per-pair boot ordering, per-pair pause, the missing-root case. This is
where the fairness policy, the boot sequence and the shutdown behaviour are decided in code, and
where the tests are genuinely new rather than mechanical (two pairs, one gate, one queue: does a
`syncnow` on pair B jump a timer-due pair A? does a 30-minute pair A bootstrap starve pair B's
watcher-driven pass? does a shutdown mid-pair-2 leave pair 1 committed and pair 3 untouched?).
Expect this phase to be as large as phase 2 and riskier. Closes: the feature, headlessly.

> **Split into three PRs — 4a (scheduler and shape), 4b (unavailable pairs), 4c (the lift) — and
> 4a shipped with departures, recorded here because 4b and 4c build on them.** 4a is runtime only:
> `config::refuse_unsupported_pair_count` still holds a resolved config to one pair, the N-pair
> constructor (`Daemon::from_pairs`) is crate-private, and no binary path reaches two pairs. What
> landed: the pure due queue (`src/due_queue.rs`, §5's order with a rotating tie-break, re-armed
> from the moment a pass *ends*); one step function that `run()` and the tests both call; boot
> through the queue; per-pair pause and per-pair cadence; shutdown as the cancel flag plus a
> `Notify`, checked before every pop; per-path watcher routing; a `pair{name=…}` tracing span on
> every per-pair line; and the `Ready`/`Unavailable` pair slot (maintainer decision M1, M1b on
> issue #102) with every sealing arm, though only `Ready` is built in production.
>
> (1) **The answers to the three questions above are tests that drive the real loop body**, not a
> pure predicate: `a_syncnow_sent_through_the_channel_jumps_a_timer_due_pair`,
> `a_long_pass_on_pair_a_does_not_starve_pair_bs_watcher_driven_pass` and
> `a_shutdown_mid_pair_two_leaves_pair_one_committed_and_pair_three_untouched` (plus
> `a_boot_sequence_is_cut_at_a_pair_boundary` through `run` in real time). Yes, no, yes.
>
> (2) **#428 re-registers the root with `watch` alone, not `unwatch` then `watch` as the issue
> named.** notify 8.2's `add_watch` walks every directory under the root and `IN_MASK_ADD`s the
> ones it already holds, so `watch` alone picks up a folder whose create event the overflow
> dropped, and nothing already watched loses its watch. An `unwatch` first buys nothing, opens a
> window with no watch at all, and fails partway on the descriptor of any directory deleted during
> the overflow (its `IN_IGNORED` was lost too, so notify still holds a descriptor the kernel has
> dropped — `inotify_rm_watch` then fails with `EINVAL`; inferred from inotify(7) and notify's
> `remove_watch`, not reproduced). If the re-walk then failed (`MaxFilesWatch`), the root would be
> left with fewer watches than before, with nothing in 4a to retry it. The real-backend test
> (`a_folder_created_during_an_inotify_overflow_is_watched_once_its_root_is_registered_again`)
> overflows a real inotify queue and shows the folder unwatched before and watched after.
>
> (3) **A pass holds its pair's `PairShared` as a field (`pair_shared: &PairShared`), not an index
> and a `pair_shared()` method.** A `&self` method borrows the whole pass, which the borrow checker
> refuses beside `&mut self.pair.connection` at the checkpoint commits; a field is the disjoint
> borrow, and it means no pass body names an index at all.
>
> (4) **The IPC `shutdown` verb does not notify.** Its `LoopCommand::Shutdown` already wakes the
> loop's wait on the control channel; the `Notify` exists for the signal task, which has no channel.
>
> (5) **An unavailable pair's `status_history` is never empty**: one in-memory entry when the slot
> is built, and one per `Sync` attempt (bounded like a ready pair's). M1b's "one entry" was read as
> "never empty", which is what keeps gui-core's `FirstRun` rule off for a reply served before the
> pair's first job pops. A paused unavailable pair seals a queued apply too: the plan for the
> unavailable arms said "re-arm only" and "same as a paused ready pair" in one row, and a ready pair
> seals it.
>
> (6) **A session that comes back mid-life now reseeds every events-driven pair**, not only the one
> whose pass reacquired it: `Daemon::event_source_generation` is bumped on reacquisition, and each
> pair full-walks once at its next steady-state pass, because every pair's stored cursor went stale
> in the same degraded window. The loop also pulls every pair in to its new, live cadence.
>
> (7) **N = 1 is wire-identical; four things change that a person could see.** Boot goes through
> the queue (same order as before: socket first, then the first pass); a cadence is measured from
> the end of the previous pass; under live events the poll and the `scan_interval` tick are one
> cadence, so there is no second pass at each `scan_interval`; and a signal wakes an idle loop at
> once. Every per-pair log line also gains a `pair{name=default}:` prefix.
>
> (8) **A drain is bounded on both channels, and the run loop's idle wait routes nothing.** The
> first version drained each channel until it was empty before it looked at the cancel flag or
> popped a job, so a tree producing events faster than they were routed held off every pass and
> every shutdown (4.2 s with the flag already set, at 50k events/s, as measured in review). Now the
> control channel's backlog at entry is taken once, and the watcher channel is taken in at most
> `MAX_DRAIN_ROUNDS` (3) rounds, each sized by the backlog counted when it begins; the flag is read
> between events. One round was the first bound and it was too tight: an echo of a pair's own
> download (#49) that arrived while the backlog ahead of it was being routed was not in that
> round's count, so the pop ran first, the pair's pass cleared `authored_writes`, and the next
> drain routed the echo as a user's edit — flipping the fresh `Synced` record to `Modified`, which
> uploads the stale file over a newer remote edit. A further round routes it before the pop; the
> cap keeps the starvation bound (at most three rounds; since each round is sized by what
> arrived during the one before, a producer that outpaces the router delays the pop by a finite
> but growing multiple of the backlog, and the shutdown flag is read between events). What
> arrives after the last round waits for the next **drain**, which the step
> reaches as soon as the job it popped has run — a step loops drain, pop, run, drain. An echo
> still in notify's own thread when the last round ends is routed after the pair's next pass has
> cleared `authored_writes`; that residual stays #425. The idle wait hands the event it received
> to the next step (`LoopInputs::carried`) instead of calling the router itself, so the router has
> one call site; it is the oldest event, routed first, so it is the cause a drain reports for
> lost events. Lost-event handling is per drain, not per notice or per round: a rescan notice or
> watcher error is noted while events are routed and settled once after the last round — one
> latch, one re-registration per affected root — where an overflow's thousands of notices each
> used to latch, warn and walk every root again.
>
> (9) **A due sweep and an explicit `Sync` for the same pair are one pass** (#193). `resync` popped
> beside an overdue scheduled sweep returns as the sweep (`Cause::Sweep`), because the daemon
> re-arms a sweep only for that cause. A `Plan` never absorbs it: a plan-only pass observes and
> consumes nothing, so the sweep stays pending for its own pop.
>
> (10) **The `pair` span is an `error_span!`** (`pair_span`, one constructor). A span below the
> subscriber's level is never created, so an info-level one removed the pair from every WARN and
> ERROR line under `RUST_LOG=warn`, which is where attribution matters most.
>
> (11) **The N-pair constructor compares pair names with the config reader's own fold**
> (`config::pair_name_key`): `Photos` and `photos` are one name in both places.
>
> **Left for 4b**, found in review and deliberately not fixed here: a notify `Err(MaxFilesWatch)`
> latches a rescan but never registers the directory it could not watch, so nothing watches it
> afterwards; a `Ready` pair demoted to `Unavailable` would publish `last_sync: None`, which the GUI
> reads as never synced; an unavailable pair's 20-entry history fills with `unavailable` entries in
> about ten minutes and needs the real history carried over; and IPC `pause`/`resume` still writes
> the metrics sidecar for an unavailable pair. A relative `local_root` is a separate, older
> problem (#431). The first four are fixed by 4b, below.
>
> **4b (availability producers) shipped, with departures.** 4b is runtime only: a resolved config
> still holds one pair. What landed: boot preparation per pair (`prepare_pair_state`, the same
> steps as before, no longer fatal), the retry from `Unavailable` (`retry_unavailable`), the typed
> `RootUnavailable` for a folder that vanishes under a ready pair, the root-identity re-watch, the
> per-root watch state (`RootWatch`), the typed `LockHeld`, and the four items 4a left for it.
> Maintainer decisions M1 (keep running, N = 1 included) and M1b (show the error, not the
> onboarding wizard) are on issue #102.
>
> (1) **A root that cannot be watched keeps its pair *ready*; it is not made unavailable.** M1's
> wording says "prepared or watched", §6.5 says degrade, and §6.5 is what is built: syncing
> without a watch is correct and only slower, so the pair scans locally on every pass and
> registers the watch again before each one, instead of dropping out of sync. The same state
> (`RootWatch::Incomplete`) carries 4a's first left-over: a watcher `Err` (`MaxFilesWatch` names the
> directory it could not add) marks the roots it names, and a failed overflow re-registration marks
> its root, so neither is left unwatched with nothing to retry it. The retry is before the pass,
> not at the error, so a burst of errors costs one registration per root per pass. Each retry
> re-walks the root (O(directories) `inotify_add_watch` calls); with the limit exhausted that is
> paid every pass, and it has no backoff.
>
> (2) **A folder that vanishes under a ready pair does not demote the pair.** §6.4's typed error
> leaves the runtime in place, and that is what keeps `seal_apply_outcome`, the latches and the
> rest of `reconcile_blocking` doing their jobs. `RootUnavailable` is raised after
> `take_apply_request` and before the event-source reacquire and the `force_full_walk` /
> `reset_index` swaps: after, so a booked apply is answered; before, so a `reset-index` made while
> the folder is gone is not spent by a pass that did nothing, and so the events-mode idle
> fast-path cannot report `Clean` and move the cursor over it. It writes no `sync_passes` row,
> spawns no child, and writes **no sidecar** — `write_atomically` used to create its directory,
> which with the default layout is inside the root, so a status write would have made the missing
> folder again (since item 13 it creates none, and the typed arm still skips the write because
> there is nothing to record). Demotion (`demote_pair`) exists and had one production caller, the first metrics write,
> which is part of preparing a pair, at boot and on a retry; review added two more causes, (8) and
> (9) below. A vanished folder with the default layout leaves an index and lock on an unlinked
> inode until the folder returns, and (8) is what happens then.
>
> (3) **A retry never creates the folder; only boot does.** `RootMode::Create` at boot, `MustExist`
> on a retry: a mount point that disappears and is re-created empty by the daemon would be
> reconciled against the remote as the user's own empty tree. The cost is that a pair
> unavailable because its folder could not be created at boot stays so until the folder exists or
> the daemon restarts; the reason text says that. An existing but empty mount point is still not
> "missing" (#426, not in this phase).
>
> (4) **`LockHeld` is fatal at boot and a failed attempt on a retry.** At boot it refuses the start
> naming the pair (`folder pair 'default': daemon already running; lockfile is locked at …`), which
> is what the per-root lock exists to stop, and it is taken before the one global lock as before. On
> a retry it leaves the pair unavailable ("locked by another process") and the daemon running.
>
> (5) **An unavailable pair publishes its real history, and a standing cause is one entry.** Its
> history is never empty (gui-core reads no `last_sync` and no history as `FirstRun`), it starts
> from the sidecar when that is readable (boot) or from the demoted runtime's history, and
> `record_repeatable_status_entry` moves the newest entry's time for a repeat of the same cause
> instead of adding one — at the 30 s cadence an entry per attempt evicted a 20-entry history in
> about ten minutes. The same rule covers a ready pair whose folder is missing. `last_sync`, the
> last plan and successful summaries, the pass history and the corpus size are carried across a
> demotion and back, so an established pair never starts publishing "never synced" (4a's second and
> third left-overs).
>
> (6) **The IPC task decides from `PairShared::has_runtime`.** It opens the approvals connection on
> first use when the flag is set and drops it when it is not, so a pair that becomes ready after
> the plane was built gets one, and `control_plane_pair` no longer fails the daemon when an index
> cannot be opened. `pause`/`resume` write no metrics sidecar for a pair that is not ready (4a's
> fourth left-over: the file belongs to whoever holds that pair's lock) and none into a directory
> that is gone.
>
> (7) **Root identity includes the directory's birth time.** `(st_dev, st_ino)` alone cannot tell a
> deleted-and-recreated folder from the old one on a filesystem that hands the freed inode number
> straight back (ext4 often does). The birth time counts only when both sides have one, so a
> filesystem that reports it on one `stat` and not the next cannot read as a replaced folder on
> every pass. A remount of the same device over the same path is still not caught (P5).
>
> (8) **State that went with the folder is detected, and the pair is prepared again** (§6.7; found
> in review). The default layout keeps the index and the per-root lock inside the folder, so a
> folder deleted and made again left the pair `Ready` on a connection to a file with no name: every
> pass failed to write its history, a new file was uploaded again on each pass, and `last_error`
> stuck until a restart. Each `Sync` job of a ready, unpaused pair now compares what it holds open
> with what its paths name before anything else (`examine_ready_pair`, item 12): the index by the
> `(device, inode)` recorded when it was opened (`PairRuntime::db_identity`), the lock by the held
> file's own (`LockGuard::is_the_file_at`). A mismatch, or a missing file, demotes the pair
> (`PairRuntime::removed_state`) and the retry **in the same job** reopens both with
> `RootMode::MustExist` — it still never creates the folder. The new runtime has an empty baseline
> when the index went with the folder, so its first pass is a bootstrap: it adopts, downloads, and
> has nothing to plan a deletion from. Three consequences. It runs **only while the folder is
> there**: a folder that is not a directory is the typed pass error's, once, and demoting too would
> write two history entries for one condition. A plan **refuses** rather than demotes (`plan_only_blocking`
> shares `removed_state`), because a plan observes and consumes nothing. And the IPC task's own
> approvals connection is replaced when the pair is prepared again (`PairShared::runtime_generation`,
> `ControlPlanePair::opened_under`): `has_runtime` goes false and true inside one job, so the flag
> alone cannot tell that task its connection is on an index the daemon let go of, and an approval
> written there would be accepted and applied to nothing. A filesystem that names one file
> differently on two looks (FUSE without stable inode numbers) cannot support the comparison; the
> index's identity is then `None` and the lock's `comparable` false, and only presence is checked,
> or every attempt would prepare the pair again, a full first pass each time.
>
> (9) **A folder replaced by an empty one, over a baseline that records files, is held** (found in
> review). 4b made a replaced folder scan at once, and an empty directory over a surviving
> baseline (an external `db_path`, or an index that outlived the folder) plans the remote deletion
> of everything it recorded — held by the approval guard by default, executed outright where the
> guard is off. In `examine_ready_pair` (item 12), before `maintain_root_watch` registers anything,
> the pair is now demoted instead, with a **standing** cause (`StandingCause::ReplacedByEmptyFolder`): the folder holds nothing the pair's
> own `ScanOptions` would keep (`index::local_tree_holds_syncable_entry`, which shares
> `visit_directory`'s predicates, so `.sync`, a trash directory, `.proton-sync.toml`, a conflict
> sidecar and anything an exclude rule hides do not count) **and** the baseline, as the planner sees
> it (`filter_base_index`), records items. Nothing is deleted. A retry re-checks the folder rather
> than only its existence: still empty means stay unavailable, restating one reason and moving one
> history entry; content means ready again, watched, and a first pass. The cause stands through the
> folder going away and an empty one coming back. The same guard runs when only the lockfile went
> with a replaced folder and the index survived elsewhere: preparing the pair again would open that
> baseline over an empty folder. The first round placed it in `maintain_root_watch`'s replaced path
> and in `demote_if_state_removed`, which left two holes (item 12). Boundaries: **a replaced folder that holds
> content, or a pair that records nothing, takes the old path** (re-watch, rescan), so an identity
> that changes with no real change — a filesystem with unstable inode numbers — costs one rescan and
> never a stuck pair; a directory entry alone counts as content (this guard is about a tree that is
> *empty*, not one that is much smaller); and boot has no identity to compare, so the same state at
> startup is #426. The replaced check compares against the directory the pair accepted as its
> folder (`PairRuntime::known_root`, item 12; the first round used the directory the last complete
> registration was on, which a registration that never completed left empty), kept through a failed
> registration, a folder that was gone for a pass and an overflow re-registration. The first round
> had to keep an overflow re-registration from recording a replaced directory as the watched one,
> or the notice would launder the replacement; with the judgement against `known_root` the watch
> state cannot launder anything, and the re-registration simply records what it watched.
>
> (10) **A folder that vanishes during a pass ends the pass unavailable** (§6.4; found in review).
> `ensure_root_available` runs again before the final commit of `execute_plan_and_commit`. The
> top-of-pass check cannot see a vanish after it: every later download is refused (its destination
> has no root) with a warning, and the pass used to end `Clean` and claim the newer event cursor, so
> the files it skipped were never asked for. Now the typed `RootUnavailable` is returned before
> that transaction: the cursor is held (the fifth cause of the one cursor policy), as are the
> index-only tail and the history rows, and the next pass takes the missing-folder path.
> Checkpoints that already landed stay, and the row they opened is sealed `failed` rather than left
> `interrupted`, which only a crash is supposed to leave.
>
> (11) **Smaller things review found.** A retry that finds the same OS error behind the folder as
> boot did keeps boot's reason (`FolderError`, `PrepareFailure::Folder`) instead of rewording it and
> adding a second history entry for one condition; the retry's own wording no longer tells the user
> to restart the daemon, which creates the folder even when it is an unmounted drive's mount point
> (#426). A plan on a missing folder answers with the sync's message (`root_availability` is the one
> definition), not a bare OS error.
>
> (12) **The second review round: a replaced or emptied folder never makes the daemon delete or
> download without an explicit user action.** The rule the round is held to, and every change
> below is that rule applied to a hole the first round left.
>
> - **One examination, before the pass, for every way a folder stops being the one the pair ran
>   on** (`Daemon::examine_ready_pair`, `PairRuntime::examine`; first in every unpaused `Sync` job
>   of a ready pair). The directory at the path is a different one (`PairRuntime::known_root`), or
>   the index or the lock went with it. Either, with a folder that holds nothing the pair's rules
>   would keep and a baseline that records items, **holds the pair** and creates, opens, locks and
>   downloads nothing. This is item 9 made true for the default layout: the first round demoted the
>   state first (item 8), and the same-job retry created `.sync`, a fresh index and a lock in the
>   empty folder and bootstrapped, downloading the whole remote into it, with the guard on or off.
>   The baseline is read through the connection the pair still holds, which works on an unlinked
>   file. A folder emptied **in place** with its state intact is not held; that is a deletion the
>   user made, and the guard treats it like any other. A state that went with a folder that has
>   content still demotes and is prepared again in the same job.
> - **The pair records its folder when its runtime is opened**, not when a watch registration
>   completes. A watcher refused from boot (`ENOSPC`) used to leave the pair with no record and no
>   hold. `last_watched` is gone; `RootWatch` is about registration and nothing else, and
>   `maintain_root_watch` no longer judges.
> - **Nothing makes the root mid-life, and nothing runs on another directory.** The executor makes
>   directories in one place (`ensure_directory_below`): the root is checked first, then the levels
>   are made one at a time and the walk stops at the root. `create_dir_all` made every ancestor, so a
>   tree deleted during a pass came back from the next download's destination holding one folder,
>   and the next pass planned the rest as deletions. The availability check compares the directory's
>   identity with the pair's (`root_availability(root, expected)`), at the top of the pass and again
>   before the final commit, so a directory swapped in during a pass, or an unmounted drive's mount
>   point that stays a directory, ends the pass `RootUnavailable` with the cursor held. A typed
>   `RootUnavailable` from an action ends the pass at once instead of counting as one failed item
>   among twenty (which ends it "abandoned"). (Item 13 found the two gaps this left — the client's
>   own staging directory, and the batched run — and closed them.)
> - **Only "not found" says something is gone.** Any other `stat` error (`EIO`, `ESTALE`,
>   `EACCES`) is `Look::Unreadable`: no demotion, no hold, no watch change, no acceptance, said once
>   per cause. A replaced folder whose content or baseline could not be read is neither accepted nor
>   held and **its pass does not start**.
> - **The advice no longer offers a restart.** With the guard off, a restart over an empty mount
>   point deletes everything remote (#426), which is exactly the case the hold exists for. A hold
>   ends when the folder holds content again (mount the drive, put the folder back), or when the
>   user runs `proton-sync reset-index --yes`: `retry_unavailable` releases the cause, the pair is
>   prepared (a fresh index where the state went with the folder, the surviving one otherwise), and
>   the pass consumes the existing `reset_index` latch before it loads the baseline. The baseline
>   is then empty, so the pass is a bootstrap: it downloads, adopts and has nothing to delete from.
>   `resync` does not release the hold.
> - **A `resync` latch is not spent by a pass that ends `RootUnavailable`.** It is latched again,
>   so the walk the user asked for still happens.
>
> **Known consequences, not fixed.** A *partly* restored folder (content, but not all of it) ends
> the hold; with an index that survived outside the folder the files not yet restored are then
> planned as remote deletions, which item 13 **withholds for approval once** whatever the guard
> says (with the default layout the index went with the folder, the pass is a bootstrap, and
> nothing is planned for deletion at all). An apply cut short by `RootUnavailable` is sealed
> `Failed` even though the deletions it had already checkpointed ran.
>
> (13) **The third review round: the timing windows are closed by construction, not one at a
> time.** Two rounds of closing individual windows did not converge (a second chunk of one download
> group, a swap between the top check and the scan), so this round states three rules that make
> the next window impossible rather than unlikely, and each is pinned by a test that failed on the
> second round's code.
>
> - **No code path a pass runs can make the root or an ancestor of it.** Every `create_dir_all`
>   reachable from a pass is gone: the client's download staging directory is made with
>   `create_dir`, one level below the directory the executor made
>   (`proton::create_download_scratch_dir`, the one place the client makes a directory — the
>   recursive form made a deleted root again from the second chunk of one group, holding only what
>   that chunk fetched, and the next job accepted it as a replacement with content and, with the
>   guard off, deleted the rest remotely), and `write_atomically` makes no directory (the sidecars
>   live beside the index, whose directory `prepare_pair_state` makes). What remains is boot's and
>   the retry's preparation: `prepare_pair_state` (the root only under `RootMode::Create`; the index
>   and lock directories after the root is found there), `LockGuard::acquire`'s lock directory, the
>   socket's directory at construction, and `paths::ensure_private_runtime_dir`'s fallback runtime
>   directory, none of them under a root a pass could reach. The root is also checked before
>   **every** download chunk, and a typed `RootUnavailable` out of the batched run's group-level
>   directory creation ends the pass at once like the per-action loop's, instead of being recorded
>   as one failed item per member (twenty warnings naming files, the folder never named).
> - **A plan is only ever derived from a scan of the directory the pair runs on, and nothing
>   executes on another.** The identity check runs right after the local scan in both
>   `try_incremental_reconcile` and `bootstrap_reconcile`, and again in `execute_plan_and_commit`
>   before its loop (after the apply comparison, before anything about the pass is mutated). The
>   window between the top check and the scan holds the keyring read, the event fetch and the cursor
>   capture — hundreds of milliseconds — and a swap landing there was scanned as the user's own
>   folder: the empty replacement planned `RemoteDelete` for everything recorded and executed it
>   before the final check could see it. A mismatch ends the pass `RootUnavailable` with no side
>   effect, the cursor held and every latch where it was.
> - **The first pass after any accepted replacement withholds every deletion for approval, whatever
>   `deletion_policy` or `[delete_approval]` says.** "Has content" is a binary test: a file manager's
>   `.directory` or an empty `lost+found/` in an otherwise empty replacement lifted the hold, and
>   with the guard off the next pass deleted the remote copy of everything recorded; a partly
>   restored folder did the same by design. `PairRuntime::force_delete_approval` is set when a
>   replacement with content is accepted (`examine_ready_pair`) and when a hold lifts on content
>   (`retry_unavailable`), and **not** by a `reset-index`, whose pass bootstraps an empty baseline
>   and plans no deletion. `decide_delete_gate` reads it, withholds through the same gate (same
>   pending list, same first-seen ages, same cursor hold), and counts what it withheld only for this
>   reason (`DeleteGate::forced`); the force ends when a pass completes with that count at zero. It
>   is carried through a demotion like `known_root`.
> - **`known_root` is back-filled and carried.** The look at open can fail (a `stat` that errs in the
>   instant after the preparation's own), and nothing back-filled the record: that pair had no
>   replacement protection for life. The first examination that can see the folder records it
>   (`Examination::Unrecorded`). And the identity is carried through a demotion
>   (`UnavailablePair::known_root`), so a promotion from **any** cause — not only a standing one —
>   is examined like a pass: `retry_unavailable` hands a plainly promoted runtime the carried
>   identity and runs `examine_ready_pair` on it. An empty replacement over a baseline that records
>   items is held, one with content is accepted with its deletions withheld once, one that cannot be
>   judged does not start. A lifted hold is not examined again (`standing_cause_status` just did) and
>   a reset is not examined at all (its pass empties the baseline the examination would read, and
>   would otherwise hold the pair the user asked to start over); `Promotion` records which.
> - **Two fail-closed arms gained tests.** `standing_cause_status`'s `Err(_) => Holds` and
>   `recorded_under_an_empty_folder`'s `CannotTell` survived every revert of the second round; both
>   are now pinned by a replaced folder that cannot itself be listed (`chmod 000` on the root, not
>   its parent, so `stat` answers and `read_dir` does not).
> - **Holds are in memory only.** A restart while held is #426 again, and a `reset_index` latched
>   earlier (while paused, say) releases a later hold with no new request — still the user's
>   explicit request, and kept. (The *forced approval* an accepted replacement puts on a pair is
>   persisted since item 14; the hold is not.)
>
> (14) **The fourth review round: the per-action rule, the plan verb, the persisted force, and
> three narrower holes.** Item 13's three rules held under twelve reverts, but two of its claims
> did not hold as stated — "nothing executes on another directory" and "a plan is only ever
> derived from a scan of the directory the pair runs on" — and both were reproduced by tests that
> failed on the third round's code. The rule is the same one, held to the letter now: a sync
> folder replaced or emptied while the daemon runs never makes the daemon delete files (remote or
> local), upload a swapped-in folder's content, or populate the folder from Proton, without an
> explicit user action.
>
> - **Nothing executes on another directory — before each action.** The executor checked the
>   root before its loop and before every download chunk, and every other arm ran on whatever
>   directory was at the path by then. A `LocalDelete` landing after a swap disposed of the
>   replacement's own files at the planned paths (for good in `permanent` mode, a whole subtree
>   for a directory), and an `Upload` pushed the replacement's bytes to Proton under the planned
>   path while recording the scan's digest. `ensure_root_available` now runs at the top of
>   **every** action of `execute_plan_and_commit`'s loop, for every arm — one `stat`, before
>   anything in the arm — and a mismatch ends the pass `RootUnavailable` with the cursor held,
>   the checkpoints that landed kept, and the current action without a side effect. The check
>   before the loop stays, because it guards the pass's own state (the published summary, the
>   withheld ages, the pending list); the per-chunk check stays, because a batched run is one
>   action whose chunks are its side effects; and the arms' directory creation keeps its own
>   check because `ensure_directory_below` is the one place a directory is made and must not make
>   the root — the creation primitive's rule, not a second statement of this one. (Item 15 adds
>   two looks, immediately before an upload's and a remote move's CLI child, and withdraws "per
>   action, literally": the look is at the start of each action.)
> - **The plan verb uses the pair's own verdict.** `examine_ready_pair` runs on `Sync` jobs only,
>   and `plan_only_blocking` asked `root_availability(root, None)` — whether *a* directory was
>   there — so a plan on a ready pair whose folder had been replaced scanned the replacement and
>   published `RemoteDelete` for everything recorded: rows the plan screen's typed-DELETE gate
>   pins approvals to (#227), which stand after the replacement is judged and would satisfy the
>   forced gate. The plan now reads `PairPass::root_verdict` (the identity against `known_root`,
>   and the unjudged latch — the sync's verdict without the sync's once-per-cause latch) before
>   its scan, and `build_plan_report` checks again right after the scan, in the one body the
>   daemon's verb and the one-shot child share (`expected`; the child passes `None` and keeps its
>   presence check). A plan on a replaced folder is sealed `Failed` with the sync's message and
>   stores nothing.
> - **The forced approval survives a restart.** `force_delete_approval` started `false` at open
>   and nothing persisted it, so a daemon restarted over an accepted replacement — the GUI has a
>   restart-daemon flow — executed the deletion the previous one had withheld, with the guard off.
>   It is one row in the pair's index now (`forced_delete_approval`, the `warm_start_state`
>   shape): written when the force is set (`examine_ready_pair`, `retry_unavailable`), cleared
>   **in the final commit of the pass that spends it** so a restart between the two cannot find it
>   standing on disk and spent in memory, loaded at open (a read that fails withholds for one pass
>   rather than deleting), and truncated by `reset_index_state`, because a start-over bootstraps
>   an empty baseline and plans no deletion for it to withhold.
> - **Preparation makes no root either.** `prepare_pair_state` looked at the folder and then made
>   `<root>/.sync` with `create_dir_all`, which with the default layout made a root that vanished
>   between the two calls again — the window `ensure_directory_below` was written to close,
>   reached on every retry. The state directories are made with the same primitive now
>   (`create_state_directory_below` → `create_directories_below`, one level at a time, stopped at
>   the root; `prepare_pair_state_after` is the seam that proves it), and `LockGuard::acquire`
>   makes no directory at all: a pair's lock directory is preparation's, the user-global lock's is
>   made at boot by its caller, never under a root. The root is made in exactly one place, the
>   `RootMode::Create` arm at boot.
> - **A plain promotion is judged before anything is prepared, from what the demotion carried.**
>   With the default layout the demotion dropped the only connection to the baseline, and a
>   promotion over an empty replacement — a `StateRemoved` demotion whose same-job retry failed
>   (the lock held by another process, `.sync` unwritable), the folder then emptied or replaced —
>   prepared the pair in it: `.sync`, a fresh index, a lock, and an examination that read that
>   index as "nothing recorded" and accepted the folder, after which the bootstrap downloaded the
>   whole remote into it. `UnavailablePair::recorded_items` is the baseline's count as the planner
>   sees it, read by `UnavailablePair::demoted` through the connection the runtime still holds
>   (which works on an unlinked file); `judge_carried_replacement` held the pair
>   (`ReplacedByEmptyFolder`) when the directory at the path was a different one from the carried
>   identity, over a non-zero count, and held nothing the pair's rules would keep — **before**
>   `prepare_pair_state`, so nothing is created, opened, locked or downloaded. A look that fails
>   prepares nothing either. (Item 15 removes the "different directory" and "recorded identity"
>   requirements, which three paths went around, and makes the count a `CarriedCount`.) The count
>   rides on the promoted runtime (`PairRuntime::carried_recorded_items`) and is read beside the
>   fresh index's own by `recorded_under_an_empty_folder`, until a pass completes on it or a reset
>   discards it; a second demotion carries the larger of the two.
> - **The root gets the index's two-look rule.** `known_root` came from one `stat` while the
>   index's identity took two: on a filesystem that names the folder differently on each look,
>   every job was `Replaced`, `force_delete_approval` was set on every pass, and with the guard
>   off every deletion was withheld for ever — a policy override nothing reported. `RootRecord`
>   has three states: `Recorded`, `Unrecorded` (the look failed; back-filled by the first
>   examination, as item 13 said) and `PresenceOnly` (two looks disagreed at open). Presence-only
>   means exactly that: no replacement is detected, nothing is held or forced for one, the
>   configured deletion policy applies as it is, the examination never records the folder, and
>   the pair says so once when it is opened. The index's and the lock's presence checks still
>   work, so a replacement that took the state with it is still judged. The watch still
>   re-registers on such a filesystem every pass, the cost it always had.
>
> (15) **The fifth review round: the promotion's judgement, a second look before two transfers, the
> work a failed look used to discard, and the guards nothing pinned.** The rule is item 14's. The
> review reproduced three ways a promoted pair was prepared in an emptied folder and populated from
> Proton, found a window the per-action look leaves open and a cost it adds, and — by poisoning the
> code one line at a time — found guards that no test pinned.
>
> - **The judgement asks about the folder and nothing else.** Item 14's `judge_carried_replacement`
>   ran for a pair that had recorded an identity, over a count above zero, for a folder that was a
>   *different directory*. Three paths went around it, each ending in `prepare_pair_state` making
>   `.sync` and a fresh index in an emptied folder and the bootstrap downloading the whole remote
>   into it: the folder **emptied in place** after a state-removed demotion whose same-job retry
>   had failed (the same directory, so "proceed") — C1; a count that **could not be read**, which
>   was `None` and was filtered out together with "never ready" — C2; and a pair whose root is
>   **`PresenceOnly`**, which was never asked at all — C3. The judgement now is: does the folder
>   hold nothing the pair's rules would keep, over a baseline that recorded items or could not be
>   counted. `CarriedCount` is `Absent` (never ready in this process, or settled since: nothing to
>   lose), `Items(n)` or `Unreadable`, so the two meanings `None` shared differ by construction — a
>   first attempt that read `None` as "assume items" held two existing tests' pairs that were
>   merely unavailable at boot. An unreadable count is held as
>   `StandingCause::ReplacedByEmptyFolderUncounted`: the same two ways out, worded without a
>   number it does not have. A count of `0` and a pair that was never ready are not judged; a look
>   that fails, a folder that cannot be searched and rules that cannot be built each prepare
>   nothing, and say so. The examination reads the same type
>   (`PairRuntime::carried_recorded_items`; `Recorded::Uncounted`), so a promoted pair whose folder
>   is emptied before its first pass completes is held by the examination itself.
> - **Before each action, and again immediately before an upload or a remote move.** Item 14's
>   "per action, literally" was the start of each action. `Upload` runs `ensure_directory` — a CLI
>   child that takes about a second — between that look and the child that reads the file, and a
>   swap inside it uploaded the replacement's bytes under the planned path; `MoveRemote` runs the
>   same call before `rename_or_move`. Both look again after it. The audit of the other arms:
>   `CreateRemoteDirectory` and `RemoteDelete` make their one CLI call straight after the top
>   look; the local arms go through `ensure_directory_below`, which looks; `Download`, `Conflict`
>   and `TypeConflict` reach their one CLI call through `ensure_parent_directory`, which looks too.
>   A swap *during* a transfer's own call cannot be closed by any look: the next action's look and
>   the check before the final commit hold the cursor for it, and the next sync judges the
>   replacement. Item 14's "nothing else in the executor checks" is withdrawn.
> - **A look that fails ends the pass, and the work before it is kept.** The per-action look is
>   one `stat`, and any error other than "not found" is no answer (item 8): the pass ends
>   `RootUnavailable`, fail closed, because carrying on would mean working on a folder nobody
>   could name. At about four looks a pass that cost nothing; at one per action an `EIO`, `ESTALE`
>   or timeout on a network mount discarded the index-only work accumulated since the last
>   checkpoint, and a first sync's adoptions are all of that kind, so a large one started over from
>   the scan. `keep_accumulated_work` commits it first, at every exit of the loop for that cause
>   (the top-of-action look, an arm that met the cause, a download run, and the check after the
>   loop). It is what the next checkpoint would have carried — derived from the scan the post-scan
>   identity check validated, with no side effect behind it — and a swap between two actions
>   already committed it through the previous action's checkpoint. The check after the loop used
>   to drop its tail as describing "a tree that is not there"; it keeps it now, which also means a
>   first sync whose folder is then replaced by an empty one, **with the state outside the folder**,
>   has a baseline that records what it adopted, and *that* is what holds the replacement (a
>   baseline that recorded nothing accepted it, and the bootstrap populated it). **With the
>   default layout the index goes with the folder** and the commit has nowhere to land (F-1, found
>   in the sixth round of review): the next job demoted with a carried count of zero and the
>   bootstrap downloaded the whole remote into the empty replacement. So a pass notes in memory,
>   when its plan exists, how many items it knew about — the baseline as the planner sees it plus
>   the adoptions it planned at paths the baseline does not hold, committed or not
>   (`PairRuntime::items_known_in_memory`) — and a demotion carries the **greater** of that and
>   what the database reports, under the same `CarriedCount` rules (a database that cannot be read
>   is still `Unreadable`, whatever the memory says). The figure is spent when a pass completes and
>   by a `reset-index`. It counts what the plan adopts, including an adoption planned after the
>   action that met the swap, so the number a held pair publishes can exceed what could have
>   landed by those. The failed action's own queue is discarded like any failed action's; a
>   commit that fails is said, and the pass ends with the cause it already had.
> - **The guards poisoning found nothing pinning each have a test that fails under their
>   mutation.** The loop-top look removed from every arm but two (`a_swap_before_an_arm_runs_nothing_of_it`,
>   one row per remaining side-effecting arm, every row run so a mutation names the arms it blinded:
>   `Upload` and `LocalDelete` are pinned by the round-4 tests, and the index-only arms have no side
>   effect to guard); a promotion
>   judged over a baseline that recorded nothing; a `PresenceOnly` open overwritten by the carried
>   identity (through a one-shot root-look seam on `PairRuntime::open`); the judgement's three
>   fail-closed arms; an unreadable force record spending the force; a carried count left standing
>   after a completed pass or after a reset whose pass failed; and the force carried through a
>   demotion, which is *not* redundant with its persisted copy when the state is inside the folder.
>   One mutation is equivalent today and said so: the arm-level branch's truncation of the failed
>   action's own queue, because no arm queues an index mutation before a call that can raise
>   `RootUnavailable`; it stays as the commit-after-side-effects invariant's structural guard.
>
> **Known consequences, kept.** On a filesystem that cannot name the folder consistently, an empty
> replacement over an index that survived outside the folder is reconciled as the user's own
> deletion of everything — held by the guard by default, executed where it is off — because that
> is what "the configured policy applies" means there, and the warning at open names it. A plan
> refused for a replaced folder stays refused until the next `Sync` job judges the replacement. A
> pair demoted for any cause (a metrics write that failed, say) and promoted over a folder that
> holds nothing, with items recorded for it, is held even when its index survived outside the
> folder and the emptying was the user's own: the judgement runs before the index is opened, so it
> cannot tell, and `reset-index` releases it. A look that fails with an I/O error ends the pass
> at that look, once per cause in the log, and the pair is tried again at its next turn.
>
> **Still not done**, deliberately: the empty mount point and the unmounted-at-boot root (#426); a
> watcher that cannot be built at all is still fatal (it is process-wide, not per root); and
> `fs::metadata(root)` on a hung network mount blocks the one main task, as the scan already does.
> (The lift itself, which this paragraph listed, is 4c, below.)
>
> **4c (the lift) shipped, with departures.** Config is the only thing 4c changes about the
> runtime's input: `config::refuse_unsupported_pair_count` and both its calls are gone, and
> `config::resolve_runtime_configs` resolves a file into one `DaemonConfig` per `[[pair]]` table plus
> a `RunMode` (`Daemon`, or `Preview { pair }`). `resolve_runtime_config` stays as the one-pair
> entry point (it errors when the file declares several). The binary hands the whole list to
> `Daemon::new(Vec<DaemonConfig>)`, the public door to `Daemon::from_pairs`, which remains the one
> place that checks the pairs agree on everything daemon-wide. Maintainer decisions M2, M3 and M4
> are on issue #102:
>
> - **M2: with more than one pair, every per-pair flag is refused at startup, naming the flags** —
>   including `--no-delete-approval`, `--no-events-driven` and `--no-warm-start`. §2's "refused by
>   rule 1" was a file rule and no code implemented a flag rule; this is that code. With one pair
>   a flag amends it, as before. `DaemonConfigInput::per_pair_flags_set` is an exhaustive
>   destructure that sorts every field into daemon-wide, mode or per-pair, so a new flag cannot
>   compile unclassified. `--full-walk` is a mode flag and reaches every pair's first pass.
> - **M3: `dry_run = true` inside a `[[pair]]` table is refused beside other pairs** — by
>   `validate_file_config_text` outright, and at startup unless `--dry-run` or `--no-dry-run` says
>   what the run is. A preview rehearses one pair and exits, so a per-pair key cannot pick the
>   process's mode. `dry_run = false` is accepted.
> - **M4: the lift ships before the GUI's multi-pair work.** The desktop app, the tray's Pause and
>   the notifications act on the **default** pair only; the tray would say "Paused" while the other
>   pairs keep syncing, approved deletions included. Documented on the website's desktop overview,
>   and logged once at daemon startup when more than one pair is configured. Exposure is limited
>   to hand-written configs: nothing in the GUI writes `[[pair]]`.
>
> Two further file rules came with it. **Beside other pairs, every table sets both `local_root`
> and `remote_root`** (in `resolve_pairs`, because no flag can supply one — they are refused — and
> because `validate_pair_roots` compares only the roots it is given, so a pair without one would
> have skipped every cross-pair rule). And **`proton-syncd --dry-run --pair NAME`** previews the
> named pair instead of the default one (§2): one pair per invocation, validated **after**
> resolution because whether the run is a preview depends on the flag, the file's `dry_run` and
> `--no-dry-run` together, an error unless it is, matched byte-exactly, and an unknown name is an
> error that names the configured pairs. Config errors stay all-or-nothing; beside several pairs an
> error from one pair's merge names the pair, beside one it is unchanged.
>
> (1) **The real-path overlap check runs before a pair is prepared, against every other pair.** The
> design said after preparation, between ready pairs. Run before, it creates nothing and locks
> nothing in a place that overlaps, it needs no pair to be ready, and exact aliasing is reported
> as an overlap instead of as the shared lockfile's "daemon already running". It is one function
> (`real_path_overlap`) with two callers: boot, where an overlap is fatal naming both pairs and,
> for every path, the written and the real form; and `retry_unavailable`, where the pair stays
> unavailable with that sentence as its reason, so a folder that was missing at boot is checked
> when it appears. Resolving a path that does not exist yet needed
> `index::canonicalize_best_effort` to canonicalize the deepest ancestor that does, where it fell
> back to a purely lexical answer for the whole path — a folder `b` under a symlink `link` that
> does not exist yet is `<target of link>/b`, and the lexical `link/b` places it somewhere it will
> never be made. That function now has two callers and the same contract.
>
> **Review round: a third caller, for a pair that is already running.** Boot and a promotion look
> once, so two running pairs that came to overlap later (`b`'s folder moved into `a`'s, a link
> left where it was) stayed `Ready`, and `a` uploaded `b`'s `.sync` as ordinary files. The same
> function is now asked of every other pair before each of a ready pair's `Sync` jobs
> (`examine_ready_pair`, through `stop_overlapping_pairs`). **Both pairs of an overlap become
> unavailable**, each with its own side's wording: stopping only the inner pair leaves the outer one
> — correctly configured, still `Ready` — uploading the inner one's `.sync`, which is already in its
> tree. The check is symmetric in what it finds (a shared folder and a shared state file have no
> inner side), so no rule about fault is needed. A scanner-side guard (never upload another pair's
> `.sync`) was weighed and not built: `is_sync_state_path` is name-based and top-level only by
> decision, and keying it on other pairs' real state directories would reach `ScanOptions`, the
> watcher and gui-core's disk walk and be recomputed whenever the layout changes.
>
> **Second review round: the running pair asks about unavailable pairs too, and stops only ready
> ones.** The first version asked only other *ready* pairs, on the reasoning that an unavailable pair
> runs nothing. That holds for the unavailable pair's own writes and not for what the running pair
> reads: an unavailable pair's folder, with the `.sync` it had while it ran, is what the running pair
> uploads once that folder is inside its tree. A pair can be down for a reason unrelated to the
> overlap (a held lock, an index that failed to open, a folder restored from a backup), and then its
> own retry can only keep it down: it cannot stop the running pair. So a running pair is asked
> against every other pair whose folder exists, and only a ready one is stopped. A neighbour counts
> only while its folder exists: a missing or unmounted neighbour has nothing in the tree, and one
> unplugged drive must not become two stopped pairs. A neighbour whose folder is missing is skipped
> whole, state files included, so a state file of its that lies in the running pair's tree beside a
> missing folder is not found until the folder is (boot refuses that layout).
>
> **Third review round: the folder, not the slot, decides whether a neighbour counts.** The existence
> filter was keyed on the neighbour's slot state (unavailable and no folder: skipped), so a *ready*
> neighbour whose folder had vanished — the usual state of a yanked drive, since a typed root error
> keeps the slot ready — still stopped a healthy pair, and `retry_unavailable` had no filter at all.
> Both callers now read one definition, `neighbours_with_a_folder`, which keeps a neighbour whose
> folder exists whatever its slot state. Boot does not use it: an overlap in the configuration is
> fatal there whether or not a folder exists yet.
>
> (2) **A test that built nested roots directly now builds them after construction.** 4a's
> `an_event_under_pair_a_is_never_tested_against_pair_bs_filters` made a daemon whose roots nest to
> prove that routing happens before any pair's filter; the constructor refuses that shape now, so
> the test sets the nesting on the built daemon. Its assertions are unchanged; routing stays a
> second line of defence, tested as one.
>
> (3) **`resolve_runtime_configs` reads the daemon-wide half first.** The old single-pair body
> evaluated a pair's roots before `proton_cli` and `socket_path`; the daemon-wide half is
> `ProcessValues` now, computed once and copied to every pair, so a file with two errors can name
> the other one first. Each error is unchanged, and no existing test pinned the order.
>
> (4) **Negative `!contains("not yet supported")` assertions stay.** The phrase has no producer
> any more, so they are vacuous; each sits beside an assertion of the real reason, which is what
> they were protecting. They were left rather than deleted so that no assertion is weakened.
>
> (5) **The GUI is unchanged, and what it does at N > 1 is a recorded fact, not a feature.**
> `ConfigDoc` writes top-level keys only, so it saves daemon-wide edits to a multi-pair file and
> refuses a per-pair edit as "two spellings of one setting" (pinned by
> `a_two_pair_file_saves_daemon_wide_edits_and_refuses_per_pair_ones`). The child `--dry-run`
> (`run_dry_run_impl`) reads its roots from the top-level keys, so for a `[[pair]]` file it has
> none. (Read from the code, not driven.) With a live daemon it asks the daemon, which previews the **default** pair; with no daemon
> to ask and a cached root from an earlier one it passes `--local-root`, `--remote-root` and
> `--db-path` beside `--config`, which M2 refuses with a message naming the flags, and the screen
> shows `proton-syncd --dry-run failed:` and that message. Phase 5a fixes the cause.
>
> (6) **`setup.sh` and `uninstall.sh`.** `config_values` returns every matching line where
> `config_value` returned the first, and `uninstall.sh` removes each pair's `.sync` rather than
> the first pair's; its plan listed one synced folder and so misreported. `setup.sh`'s
> "your existing config syncs X, not Y" warning still names only the first pair; setup.sh writes
> single-pair configs, so a multi-pair file is one a person made.
>
> **§8a was not measured.** Two pairs on one volume make each other fall back to a full walk
> whenever an event names a node under neither's indexed parents — the cost this ADR already
> described, now live for N > 1. It was not measured against a live account for this change. The
> procedure: run the daemon with two pairs on one volume at `RUST_LOG=info`, make changes in pair
> A's folder only, and count `event-driven pass fell back to a full-tree snapshot` lines per
> pair — each carries its `pair{name=…}` span — over the same period for one pair alone and for
> both. If pair B's count tracks pair A's activity, §8a's candidate (a) is worth its own ADR.

**Phase 5 — GUI (large, and larger than the issue assumes).** Splits into three, and they are worth
tracking separately because only the first is mechanical: (5a) pair-index `RuntimePaths` and the
~14 commands that follow, plus the selected pair in `gui.toml` — mechanical; (5b) `ConfigDoc`'s
array-of-tables API and the "promote to `[[pair]]` form" rewrite driven by phase 1's `KeyScope` —
new surface, no analogue in the file today; (5c) **the selector itself, which starts in
`docs/design-v2/Drive Sync.dc.html`, not in `chrome.js`** — re-draw, `fidelity:extract`, regenerate
51 frames, fixtures, `data-fid` mappings, copy deck, and re-check the five settled frames for hue.
Plus the tray/notification aggregation semantics (§6), which are two decisions and little code.
Closes: the feature for users.

> **Phase 5a-2 shipped (selection, status shape, capability gate; no pixel moves for one folder), with
> departures.** (1) **The selection is a field of the GUI's `RuntimePaths` (`selected`), not a separate
> managed `GuiSelection`.** `resolve_at` loads it from the `gui.toml` beside the config path, so every
> re-resolve (a save, a restart) re-reads the one file that holds it instead of each having to carry it
> across; `select_pair` is the only writer (file first, memory second, then a `pair-selected` event).
> (2) **A write names its pair, as a required `String`** (`pause`, `resume`, `sync_now`, `resync`,
> `approve`, `deny`, `keep`, `run_dry_run`, `apply_plan`, `resolve_conflict`); a read takes
> `Option<String>` and means the *selected* pair when it names none. The two are `Ask::Named` and
> `Ask::Selected` — a write has no way to reach the selection. `tray_action` is `Ask::Default` until
> phase 5d. (3) **The selection counts only once the daemon is known to read a selector**; before that,
> and against an older daemon, every request is unaddressed, and the first read re-asks addressed once
> its own reply has shown the daemon can. (4) **`deny` is behind the gate with `approve`, `keep`,
> `resync` and `apply`**: a fresh unaddressed `status` before any of them for a non-default pair, and a
> refusal unless the daemon still reads the field and still runs that pair. (5) **A reply that names no
> pair is dropped, never drawn as an outage.** A selection read falls back to the default pair and says
> so in `pair_unknown`; anything else is refused. `pair_unknown` rides on a payload whose `state` is a
> placeholder (`DaemonState` has no variant for "that pair does not exist"): phase 5c should give the
> screen a typed state. (6) **`heroStateOf` takes `drawsFirstRun`**: the window passes `pairCount >= 2`,
> the tray `true`, so at one folder the main screen behind the first-sync dialogs is what it always was
> (D2) and the tray's own copy of the rule is gone. (7) **The tray panel is pinned to the default pair**
> (the store's `reply` mode) so it never shows the selected pair beside rows that act on the default
> one. (8) A pair's withheld deletions are filed in the same publish as its status, so a switch never
> shows an empty queue. Held by two browser gates in the `fidelity` job: `check-n1-identity.mjs` (every
> frame renders the same bytes when the daemon lists one pair) and `check-pair-routing.mjs` (a write
> acts on the pair it was drawn for, with a reply held open while the selection moves).
>
> **Review round on 5a-2, recorded because each changes a rule above.** (9) **The window's selection moves
> only on a reply that describes the pair it selects.** Rust stamps `selected` when it builds the payload,
> so a read that left for pair A before `select_pair(B)` and landed after it describes A and says B;
> taking that `selected` moved the window to a pair with no status and drew "unreachable". (10) **A
> selection read that is not understood is retried unaddressed**, like one that names no pair: a daemon
> replaced by an older one mid-session answers the addressed read without a `pair`, and filing that reply
> is what shows the capability is gone. Writes still never retry. (11) **A conflict is named by its pair
> and its path**: two folders can both hold a conflict at `note.txt`, and keyed by path alone a late read
> for one was accepted as the other's and drawn on the card where the person chooses which version to
> destroy. (12) **The n1 gate pins the clock** (`clock-pin.mjs`) and counts what its injection reaches:
> fixtures froze `Date.now()` per page load, so eight frames that print an absolute time rendered
> differently from one URL when a minute fell between loads. Known gap, not closed here: the tray
> panel's first poll, before the roster is known, names no pair, and Rust answers a read that names none
> about the *selected* pair — so with a non-default pair selected the panel shows it for one poll
> before the pin takes hold. Nothing can select a pair yet (PR 6).

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
