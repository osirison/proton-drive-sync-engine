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

## Tab 1 — Folders

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
