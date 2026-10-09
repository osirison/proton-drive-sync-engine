# Behaviour and state

## Daemon interface

Unchanged. Keep `gui/src/js/api.js` and the existing socket contract; this redesign is a
presentation layer over the same fields.

**Fields consumed**, in the three tiers they actually come from — they are not one flat payload,
and the redesign's summaries depend on knowing which is which:

*From the daemon's `ControlResponse` (the `status` socket reply):* `status` · `paused` ·
`syncing` · `reconcile_seq` · `pending_changes` · `message` · `last_sync_epoch_secs` ·
`last_error` · `last_plan_summary` · `last_successful_sync_summary` · `status_history` (last 20) ·
`pending_deletions` · `config` · `activity`.

The three counts — `conflicts`, `destructive_actions`, `skipped_unsupported` — live **inside the
nullable `last_plan_summary`**, not at the top level, so a null summary means *unknown*, not zero
(render em-dashes, never `0`). `config` is a `RunningConfigInfo` carrying only `local_root`,
`remote_root` and `db_path`.

*Derived by the GUI*, not reported by the daemon: `state` — the `get_status` command wraps the
reply into a `StatusPayload` whose `state` is one of `running` / `idle` / `paused` /
`authExpired` / `unreachable` / `firstRun` (camelCase on the wire).

*From the config file* via `read_config`: `scan_interval_secs` · `events_driven` (**not**
`event_driven_reconcile` — that key does not exist in the engine) · `include` / `exclude` ·
`proton_cli` · `socket_path` · `delete_approval_remote` / `delete_approval_local`.

**Fields consumed from `--dry-run`:** `total` · `uploads` · `downloads` ·
`remote_directories_created` · `local_directories_created` · `local_moves` · `remote_moves` ·
`auto_links` · `conflicts` · `type_conflicts` · the action list (`action`, `path`,
`entity`, `remote_id`, and a destination path for moves/conflicts).

The redesign still needs all of these; it just stops displaying them as the primary content. They
surface in the *Details* panel and drive the plain-English summaries.

## New capabilities the design assumes

Flag these as product decisions, not UI details. Where one is unavailable, the fallback is given.

| Need | Used by | Fallback if unavailable |
| --- | --- | --- |
| Per-file state + history query | Activity file lookup | Show verdict + both side cards; omit the history block |
| Byte totals per direction, per window | Main screen footer, Activity | Omit the footer totals line |
| Diff summary in prose ("added a line") | Conflicts version cards | Metadata row only — do **not** fall back to the raw diff |
| Live match count per skip rule | Settings › What to skip | Show the pattern list without counts; keep the stale-rule marker if resolvable |
| Filtered apply (plan minus deletions) | Plan › `Run it without the deletion` | Hide the button |
| `full_scan_schedule` (weekly/monthly + time) | Settings › schedule | Keep `scan_interval` but present it as "every N minutes" in plain language |
| `deletion_policy` | Settings › Deletions | Hide the tab; behaviour stays "ask every time" |
| `notify_policy` | Notifications | Hide the section; default to the four events |
| Free-space check on the local root | Onboarding step 2 | Omit the "You have 214 GB" clause |
| Distro detection | CLI-missing screen | Show tarball instructions |

## App state model

```
first-run ──▶ folders ──▶ plan ──▶ first sync ──▶ consent ──▶ running
                                                                 │
        ┌────────────────────────────────────────────────────────┤
        ▼                    ▼                  ▼                ▼
     settled  ◀──▶  syncing        needs-decision        paused
        │                                │
        └──────────▶ unreachable ◀───────┘
```

**needs-decision is additive, not exclusive.** Conflicts and withheld deletions coexist with
settled, syncing and paused. The hexagon shows the *transfer* state; the status chip and the
attention band show decisions. Only when nothing is transferring does the hexagon itself take the
decision form.

**unreachable** is entered after a failed pass and retry; it does not stop the queue. Everything
keeps waiting. The design says so on every surface.

## Which folder the window is about (two folders or more)

The state model above is one folder's. With two or more, the window is about **one of them at a time**
— the one the header's pill names — and `select_pair` (Rust holds the choice, so the window and the tray
panel read one value) is how it changes (#102 phase 5c-1, DEVIATIONS §108).

- **Everything on screen is that folder's.** The chip, the attention band, the hero, Deletions,
  Conflicts, Plan, Activity and Settings read it; another folder's waiting decisions are the pill's ring,
  a failed or unavailable folder is its solid dot (a paused one marks nothing), and the list says which and
  how many.
- **A switch is a change of shape, not an update.** What describes the folder that was left is dropped
  before anything is drawn from it: the open conflict and the position in the queue, an armed typed-`DELETE`
  field (it is about one row of one folder, and a row of the same path in the other folder must not answer
  to it), a plan rehearsed for the other folder, a half-typed lookup against its index, the file counts of
  its skip rules. What is KEPT is what is keyed by folder: each folder's staged Settings edits, the
  decisions already answered, the settings read for each. Nothing typed is lost by looking at another folder.
- **A reply is about the folder it was asked of.** A status read, a conflict scan, a rehearsal and a
  decision each carry the folder they were issued for and are filed under it; one that lands after a switch
  is a true fact about a folder that is no longer on screen, and changes nothing on the one that is. A
  button acts on the folder it was DRAWN for, not on whichever is selected when it is pressed.
- **The folder the window remembered may not be running.** The remembered choice lives in `gui.toml`; a
  daemon that has not restarted onto a newly added folder does not run it. The window shows the default
  folder and says so (the notice block, `03-main-screen.md`); it does not rewrite the choice, which applies
  again the moment the daemon runs that folder.
- **At two folders the first-run wizard never opens** (its `Next` writes top-level roots a `[[pair]]` file
  refuses), and a folder that has never synced is drawn as such rather than as `Everything is up to date`.
- **A pause the daemon could not save** (`pause_unsaved`): the pause TOOK EFFECT; what is not saved is that
  it survives a restart. The window says so in the notice block, from the hero's own press and from a tray
  row alike (the panel is dismissed before the reply arrives, so Rust tells the window). The notice is kept
  per folder and ends when it stops being true — the next press, a status that shows the other pause state,
  a daemon that stopped answering (`03-main-screen.md`).

## Adding and removing a folder (#102 phase 5c-2, DEVIATIONS §110)

The two dialogs that change which folders there are (`08-settings.md`) have a state of their own, and these are the
rules it keeps.

- **A check belongs to the text it was made for.** `Check folders` (the engine's answer and both sides' price) is
  filed with what was typed; one character later it is stale, the button is `Check folders` again, and Enter on a
  field runs the check — never the add.
- **The order is the design:** one write (`add_pair`), then a restart only if the daemon was running, then a wait
  for the daemon's OWN list to name the folder (the file names it from the write; only the daemon's list says it
  runs), then the selection, then the merge dialog watching the new folder's own pass counter. The selection moving
  without a click is that one step, and everything before it carries the name captured when the button was pressed.
- **A dialog with something in flight cannot be left.** While the add is writing, restarting or waiting, Esc and
  the ✕ do nothing and `Cancel` is gone; while a removal runs, the same. The file is written before the restart,
  and a dialog closed between the two would leave a folder added and nothing watching for it to appear. A restart
  that did not work ends the wait with the Settings save's own five sentences and a `Restart it now`, which is also
  latched for the Settings bar.
- **A daemon that never lists the folder is said so** after a bounded wait (twenty polls), not waited for for ever,
  and the folder stays in the file.
- **The first-run takeover never takes the window beside a new folder.** The new folder has not synced; at two
  folders that is a hero (`nothing synced yet`), and the merge dialog is the answer.

## Transitions

| Trigger | Effect |
| --- | --- |
| Pass starts | Seam, side labels and transfer columns fade in `320ms ease-out`. Hexagon crossfades to syncing `220ms`. It does not move or rescale. |
| Pass ends, nothing waiting | Reverse. Screen returns to silent. |
| New decision arrives | Attention band slides in from the bottom `220ms ease-out`; the status chip changes; the seam shortens to stop above the band. |
| Decision resolved | Band row collapses `180ms`; when the last one goes the band leaves and the seam extends again. |
| Pause | Hexagon → dashed `opacity:.55` `220ms`; transfer rows stay, greyed; headline and button copy swap. |
| Proton unreachable | Hexagon → struck; retry countdown ticks in mono; nothing else changes. |
| Conflict resolved (Conflicts screen) | Crossfade to the next conflict `220ms`; header and buttons stay put. |
| Switch of folder (two or more) | Nothing animates and nothing moves: the header keeps its pill, the screen is rebuilt for the other folder in one step. The list closes first. |
| Theme change | `120ms` colour transition on background/border/text; no layout change. |

`prefers-reduced-motion`: no travelling segments (static coloured outline at 40% opacity), no
`breathe`, no `blip`, no slide-ins — opacity only. Progress bars still animate.

## Gates and confirmations

| Action | Gate |
| --- | --- |
| Keep / Keep both / Keep it | **None.** Immediate. It is the reversible direction. |
| Move to Proton's Trash | None — recoverable |
| Delete permanently (either surface) | Type `DELETE`, case-sensitive, clears on blur. Then a full-window confirmation naming what is lost. |
| Run a plan containing a deletion | Type `DELETE` in the footer bar. Only shown when the plan actually deletes. |
| Discard a conflict version | No typed gate, but the button wears the decision outline and the note says it can't be undone from here. |
| Save settings | None, but the daemon may refuse — surface its reason and say the old settings are still running. |
| Never ask about deletions | No gate at the control; the card is tinted destructive. |

## Interactive states

- **Hover:** surfaces step up one level (`#0D0E11 → #101216`); borders step up
  (`#23262D → #2E323A`); text one tier brighter. `140ms ease-out`.
- **Press:** no transform; background steps up one more level.
- **Focus:** `2px` outline in `#3B82F6` (dark) / `#1D4ED8` (light) at `2px` offset. Every
  control must be keyboard-reachable — this is a desktop app.
- **Disabled:** primary `#2A2E36`/`#6D7783`; destructive `rgba(255,59,59,.1)`/`#8A5A5A`; both
  `cursor:default`, no hover response.
- **Selected:** in radio cards, `border:1px solid #2E323A; background:#101216` plus the `4px` ring
  dot. In pill tabs and day chips, the inverted fill.

## Keyboard

The folder list (two folders or more) is its own small keyboard: `↓` on the pill opens it, `↑ ↓ Home End`
walk it, `Enter` chooses, `Esc` closes it and returns to the pill.

`Ctrl F` focus the Activity lookup · `Esc` cancel a confirmation or close a dialog ·
`← →` move between conflicts · `Ctrl S` save settings · `Ctrl ,` open Settings ·
`Ctrl W` close the window (keeps syncing) · `Ctrl Q` quit (stops syncing).
`Ctrl W` and `Ctrl Q` having different consequences is exactly why the tray labels spell it out.

## Empty and error states — all specified

| Screen | Empty | Error |
| --- | --- | --- |
| Main | settled state | unreachable hexagon + waiting count |
| Conflicts | `Nothing left to decide` | — |
| Deletions | `Nothing waiting to be deleted` | — |
| Plan | safe-plan variant | dry run failed → show the daemon string, offer `Check again` |
| Activity › files | `Nothing has moved in the last hour.` + flat line | — |
| Activity › lookup | no match → `No file by that name in your sync folder.` | — |
| Activity › passes | fewer than 20 → shorter chart, no padding | the failed pass expands inline with its exact error |
| Settings | no skip rules → hide the list, keep the add row | save refused dialog |
| Onboarding | Proton folder empty → counts read `0` and copy becomes "everything here will go up" | CLI missing screen |

## Testing checklist

- [ ] Hexagon is **pointy-top** at every size; no nested circle or ring
- [ ] Hexagon does not move between states of the same screen
- [ ] Footer's four doors never move or reorder
- [ ] Seam absent when settled; stops above every full-width band
- [ ] Every centred element on the seam has an opaque background mask
- [ ] No colour anywhere on a settled screen
- [ ] Keep is the highest-contrast button on Conflicts and Deletions
- [ ] Solid red appears only on the armed confirmation and permanent-deletion markers
- [ ] No destructive action in any notification
- [ ] `Close window` / `Quit` sub-labels present in the tray
- [ ] Every window fits `1040×764` with no clipping and no overflow onto the footer
- [ ] Tray glyph distinguishable in one colour at 16px, all five states
- [ ] Both themes: contrast checked, gradients theme-aware
- [ ] `prefers-reduced-motion` honoured
- [ ] Daemon error strings shown verbatim, never paraphrased
