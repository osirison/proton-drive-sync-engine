# Settings

**Route:** footer nav → *Settings*. **Purpose:** change what syncs, how often, and how much say
you get over deletions.

## What changed and why

The old screen was the daemon's config file with labels: `Scan interval (seconds)`,
`Event-driven reconcile` ("Use Proton's volume-events stream for fast incremental reconcile…"),
and glob lists explained by a paragraph about include/exclude precedence. Every field was named
after the key it writes.

Now every control says what it does **to your files**, with the config key underneath in mono. The
skip rules show **how many files each rule is currently hiding** — the number that makes a
forgotten rule findable.

## Tabs

Pills, same treatment as Activity: **Folders · What to skip · Deletions · Advanced**.

Title block on every tab: `Settings` 26px/600 /
`Changes here take effect on the next sync. Nothing is written until you save.`

## Footer action bar (all tabs)

Left, 12px, `max-width:430px`:
`Saving writes only what you changed. Your comments and anything the app doesn't understand are left alone.`
When a pending change has a cost, this is replaced by a `#FF9F1C` line stating it, e.g.
`One rule removed — 2 files, 3.1 GB will start syncing.`

Right: `Discard changes` (quiet) + `Save` (primary; `#2A2E36`/`#6D7783` disabled until dirty).

**At two folders or more** a staged change the daemon reads has one more cost, and it is the same for every
folder, because a save restarts the one daemon: the line becomes
`Saving restarts syncing for all two folders, briefly. Anything running now stops, and folders you paused stay paused.`
(`#FF9F1C`, 12px, `max-width:600px` — two sentences — and following a rule-removal cost when both are staged).
It says what is true and no more: a folder's pause is kept by the daemon across a restart (#441), and a pause
set while a folder was unavailable is the known gap (#442) the line does not mention. It is drawn only while a
daemon-config change is staged — a policy kept in the app's own file restarts nothing — and never below two.
(Drawn: `8a Save two folders`, a 600px crop of the bar; DEVIATIONS §110.)

## Tab 1 — Folders

**At two folders or more** (nothing below two; DEVIATIONS §110) the tab opens with the list of folders, above
the settings it chooses between:

- **Your folders** (10px label) over a panel, `border:1px solid #1A1D22; border-radius:13px; background:#0D0E11`,
  one row per folder **in the settings file's order** (the first is the default folder). A row is
  `padding:11px 18px; gap:14px`, the folder on screen on `#101216` with a filled `8px` `#F2F4F7` dot: the
  **name** (14px/600) over the two paths in mono 11px `#6D7783` (`~/Documents ⇄ /Drive/Documents`), the word for the
  folder's state (12px `#828B98`, the folder selector's own words, or `not running yet` for a folder the file
  lists and the daemon does not run) and `Remove`. The name block is one button: a click on it is `select_pair`,
  the same choice the header's pill makes.
- Under it, 12px `#6D7783`: `The settings below are for documents. Click another folder to change its settings
  instead.` — **Settings edits the selected folder**; everything under the list is that folder's, and what is
  typed for one folder is kept for it when another is chosen.
- `Add folder…` (a `Choose…`-sized button), which opens the add dialog below. The ⋯ menu carries the same
  entry at **every** count; it is how a person with one folder finds the second.
- The settings themselves are unchanged and sit in their own block under the list, so the seam keeps the
  position it was measured at. **Advanced** says `Applies to all folders.` under the command, the log level
  and the socket — the settings the daemon reads once for every folder (`include` and the conflict suffix follow
  the selected folder).
- **Remove** asks first, in a 600px dialog (below). The last folder cannot be removed: the engine refuses an
  empty list, and with two or more there is always another.

(Drawn: `8a Folders list`, a 600px crop of the list. The rest of the tab is `8a Settings`.)

**The pair being kept in step** (10px label), then a seam block. The seam here is short:
`position:absolute; top:44px; height:86px; left:50%` — sized to span only the two inputs, because
a full-width line sits below it.

| Side | Spec |
| --- | --- |
| Left | `This computer` label `#FF9F1C`. Input (`flex:1`, `padding:11px 13px; border-radius:10px; border:1px solid #23262D; background:#0D0E11`, mono 13px) + `Choose…` button. Helper 12px `#6D7783`: `12,480 files, 41.2 GB in here today. Changing it starts a fresh merge — nothing gets deleted.` |
| Right | `Proton Drive` label `#22D3EE`, `text-align:right`. Input right-aligned, full width, no browse button. Helper right-aligned: `Folder on your Proton Drive. Must already exist.` |

**How often it checks** (label, `margin-top:20px`) — two panels,
`border:1px solid #1A1D22; border-radius:13px; background:#0D0E11; padding:13px 18px`:

**Panel 1 — live updates.** `Notice changes the moment they happen` 14px/600 /
`Proton tells the app when something changes on another device, so it syncs within seconds.`
12.5px `#99A2AE` / `events_driven` mono 11px `#6D7783`. Toggle on the right:
`44×26px` `border-radius:99px`, on = `background:#F2F4F7` with a `20px` `#0A0B0D` knob at
`top:3px; right:3px`.

**Panel 2 — the full sweep schedule.** Replaces the old `scan_interval` number field entirely.
- Header row: `Compare everything, top to bottom` 14px/600 /
  `A full check of all 12,480 files as a safety net. It's slow, so it runs on a schedule rather than constantly.`
  Right: a **Weekly / Monthly** segmented control — `display:flex; gap:3px; padding:3px;
  border-radius:10px; background:#0A0B0D; border:1px solid #23262D`, each option
  `padding:6px 14px; border-radius:7px`; active `background:#F2F4F7; color:#0A0B0D; 600`,
  inactive `transparent` / `#99A2AE`.
- Control row `margin-top:11px; padding-top:11px; border-top:1px solid #16181D; gap:10px`:
  `Every` (12.5px `#828B98`) → seven day chips `width:42px; padding:6px 0; border-radius:8px`,
  inactive `border:1px solid #23262D; color:#99A2AE`, selected
  `border:1px solid #2E323A; background:#F2F4F7; color:#0A0B0D; 600` → `at` → a time stepper
  (`−` / mono 13px `03:00` in a `48px` centred span / `＋`, buttons `28×28`) →
  `flex:1` → right-aligned mono 11px `full_scan_schedule · weekly sun 03:00`.
- **Monthly variant:** `On day` + a `repeat(10,1fr)` grid of day chips (`padding:5px 0;
  border-radius:6px`, mono 11px), the same time stepper, key line
  `full_scan_schedule · monthly day 15, 03:00`, and
  `Months without a 15th are skipped to the last day.` 12px `#6D7783`.

**Run one now** (label, `margin-top:18px`) — its own section, per the schedule being separate from
the manual trigger: `Full sweep now` 14px/600 /
`Takes about 4 minutes; syncing keeps working. Last one 2 days ago — nothing was out of step.` /
`Sweep now` (`#101216`/`#2E323A`/`#E8EBF0`/600, `padding:10px 20px`).

## Tab 2 — What to skip

Intro 13.5px `#99A2AE` `max-width:640px`:
`Anything matching a rule below stays on this computer and is never copied to Proton Drive. Rules are matched against the path inside your sync folder.`

**Your rules** label + right-aligned mono 11px `hiding 4 files, 3.1 GB in total`.

Rows `padding:13px 2px; border-top:1px solid #16181D; gap:14px`:
`pattern (mono 13px #F2F4F7, width:180px) · effect · Remove`

| Pattern | Effect (12.5px `#C9D0DA` + mono 11px `#6D7783` beneath) |
| --- | --- |
| `*.tmp` | `Skipping 2 files right now` / `exports/draft.tmp, exports/render-final.tmp` |
| `video-raw/**` | `Skipping 2 files, 3.1 GB` / `added 14 Jul · the folder still exists on this computer` |
| `old-backups/**` | `Matching nothing` / `no such folder here any more — safe to remove` — whole row at `opacity:.62` |

**The live match count is the point of this tab.** A stale rule is dimmed and marked removable; an
active one names the files it is hiding.

Add row: input (`placeholder="Add a rule — e.g. *.psd or scratch/**"`, mono 12.5px) + `Add`.

Bottom, `margin-top:auto`: a neutral panel — `⊘` `#626B78` +
`Two more files can't be synced no matter what — a socket and a shortcut. Nothing you can change here.`
+ `See them` (opens the dialog from `07-activity.md`). Then mono 11px:
`The app's own .sync folder is always skipped and can't be added here.`

The old include-list and the precedence paragraph are gone. If include globs must stay, they belong
behind *Advanced* — most users only ever exclude.

## Tab 3 — Deletions

`When a file is deleted` 18px/600 /
`Deleting on one side would normally delete it on the other. This is how much say you get.`

Three radio cards, `border-radius:12px; padding:14px 16px`. Selected:
`border:1px solid #2E323A; background:#101216` with a `15px` dot
(`border:4px solid #F2F4F7`). Unselected: `border:1px solid #1A1D22; background:#0D0E11`,
`1.5px` `#3E454E` ring, title `#C9D0DA`. Bodies are 12.5px, `padding-left:26px`.

1. **Ask me every time** — `recommended` badge (mono 10.5px in a bordered pill).
   `Deletions wait in a queue until you approve them. Nothing disappears behind your back.`
2. **Only ask about permanent ones** —
   `Deletions that go to Proton's Trash happen automatically. Anything removed from this computer for good still waits for you.`
3. **Never ask** — card gets `border:1px solid rgba(255,59,59,.3);
   background:rgba(255,59,59,.04)`, title `#FF9C9C`, body `#C9D0DA`:
   `Deleting a file on either side deletes it on the other immediately, including permanently from this computer.`

Key line: `deletion_policy · applies to both directions`. New setting — needs daemon support.

### Second panel — what a deletion does

**Not drawn in any frame** (DEVIATIONS §100a): `8a Deletions tab` has one panel and the tab now
carries two. The panel above decides whether a deletion **waits**; this one decides what a local
deletion **does** when it goes ahead. Same tab because they are the same subject; separate panels
because they are separate questions, and neither changes the other's meaning.

`What deleting does to your copy` 18px/600 /
`This is about files on this computer. Anything deleted on Proton Drive always goes to Proton's Trash.`

Two radio cards, the same pattern as the panel above:

1. **Move them to the trash** — the default, and it carries the same `recommended` badge as
   *Ask me every time* above (`SETTINGS.recommended`, already drawn in `8a Deletions tab`).
   `Deleted files go to this computer's Trash, where you can restore them from your file manager.
   They keep taking up space until you empty it.`
2. **Delete them permanently** — plain card, **no destructive tint**, unlike *Never ask* above.
   The difference is what each one costs: *Never ask* takes a person out of the loop for every
   future deletion, while this is a considered choice about disk space whose consequence its own
   body states. A red card here would be the same overstatement this change removes from the
   Deletions screen.
   `Deleted files are removed from the disk straight away, freeing the space. There is no trash to
   get them back from.`

Key line: `local_delete_mode · applies to this computer only`.

Turning this to **permanent** is what restores every warning on the Deletions screen — the
`Permanent · this computer` header, the destructive card tint, and the typed-`DELETE` gate. Those
were not removed; they are conditional, and this is the condition (05-deletions.md, *Two disposal
modes*).

## Tab 4 — Advanced

Not drawn. It holds: include globs, the socket path, the CLI binary path, log level, conflict
suffix, and a *Reset the index* action. Use the same panel pattern; keep the plain-language title +
mono key structure. Anything genuinely dangerous here gets the typed-word gate.

## Add a folder (720px dialog, two sides side by side)

Opened from `Add folder…` and from the ⋯ menu. Not the first-run takeover (which writes the implicit single
pair and is shut at two folders): a small question, and then the merge dialog first-run shows, for the new
folder. (Drawn: `8a Add folder`, DEVIATIONS §110.)

`Add a folder` 18px/600 / `A folder on this computer and a folder on Proton Drive, kept identical.` and a ✕.

- **Name** (10px label) and a mono field, `padding:11px 13px`, full width. It **follows the folder's own name**
  (`~/My Photos` → `my-photos`, made by the app from the engine's own charset and never a second copy of it)
  until it is typed in, and then it is the person's. Under it, 12px `#6D7783`:
  `How this folder is named in the app and in commands, such as proton-sync --pair photos. It can't be changed afterwards.`
  — or, when the engine refuses the name, **the engine's own sentence, verbatim, in mono** (`#FF9C9C`), asked as
  the name is typed.
- **This computer** (`#FF9F1C`) with `Choose…`, and **Proton Drive** (`#22D3EE`, right-aligned), in two columns.
  After `Check folders` each says what it holds in mono 11.5px: `1,204 files, 3.4 GB` and `1,190 files` — the Proton
  side reports a count and **no size** (a remote listing exposes none), and a walk that stopped at its bound or could
  not read a directory says `at least`.
- **Skip rules** (10px label) — `Optional. Anything matching a rule stays on this computer and is never copied to
  Proton Drive.` — a field and `Add`; the rules ride in the same single write that adds the folder.
- When the folder already holds `.sync/sync_index.db`, an amber block (the never-synced band's tint) says so,
  **before** anything is written, in the maintainer's words: `This folder already holds sync history from an
  earlier setup (…); adding it resumes from that history, so anything changed since may show up as deletions to
  approve. To start fresh instead, run proton-sync reset-index --yes --pair photos after adding.` The app cannot
  reset an index.
- Foot: 12px `#6D7783` `There is no preview for a new folder. Its first sync starts as soon as you add it, so what
  differs between the two sides is only known once it runs.` — and **nothing about what the first sync will do to a
  file**: no plan was made, and the dialog claims none. `Cancel` and the primary button.

**The primary is two buttons and a person sees them in order.** `Check folders` asks the engine whether the add
would go ahead (`check_add_pair`, the same function the add itself runs, so the two cannot disagree: the name, the
roots, a relative local folder — #431 — and a real-path overlap through a symlink) and prices both sides
(`probe_folder`; the remote side asks the daemon behind its one gate, #23). Nothing is written. It becomes
`Add folder` only for the text it was made for: **editing a field puts `Check folders` back**, and Enter never adds
what was not checked. A side that could not be measured says why (`Couldn't be measured — …`, the reason verbatim)
and does not block: a daemon that is busy says nothing about the folder.

`Add folder` writes ONCE (`add_pair`: promotes an implicit file to `[[pair]]` tables, `default` first, and carries
the staged rules), restarts the daemon only if it was running, **waits for the daemon's own list to name the
folder**, selects it, and opens `9a First sync` addressed to the new folder. That dialog has **no plan footer** —
nothing rehearsed this merge — and watches the NEW folder's own pass counter, read from its own summary. While the
add is in flight the dialog cannot be left (Esc and ✕ do nothing), because the file is written before the restart.
A restart that did not work says so in the Settings save's own five sentences, with `Restart it now` there and on
the Settings bar.

## Remove a folder (600px dialog)

`Remove documents?` 18px/600, then four sentences, 13px `#99A2AE`, `gap:10px` (the fourth only when the folder is
the first, because the first is the default one):

1. `Syncing stops for documents.`
2. `Nothing is deleted on this computer or in Proton Drive.`
3. `Its sync history is moved aside to ~/.local/state/proton-sync/removed-pairs, so adding the folder back later starts fresh.`
4. `photos becomes the default folder: commands that name no folder, and older versions of this app, will mean it.`

`Cancel` is the **primary** (the safe choice is the loud one) and `Remove folder` the quiet button beside it. The
dialog then shows the command's actual reply in place of the sentences — `was removed from the settings.`, then the
account of where the history went (moved to X, nothing to move, or **pending, with the reason**), verbatim — and
`Done`. (Drawn: `8a Remove folder`, the first folder; the answer is the command's own words and is not drawn.)

## Save refused (600px dialog)

The existing daemon rejects a config write if its own parser would refuse the result. Surface that
properly:

`34px` warning hexagon `#FF6B6B` + `That folder doesn't exist on Proton Drive` 16px/600 +
`Nothing was saved — your old settings are still running. Create the folder on Proton Drive first, or pick a different one.`
12.5px `#99A2AE` + the daemon's reason in a mono 11.5px box:
`remote_root: /Drive/Archive2026 — not found` + `Go back and fix it` (strong) /
`Create it on Proton Drive` (quiet).

**"your old settings are still running" is the important sentence** — a failed save must say what
state the system is in.
