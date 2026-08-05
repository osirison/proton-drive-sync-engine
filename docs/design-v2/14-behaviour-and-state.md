# Behaviour and state

## Daemon interface

Unchanged. Keep `gui/src/js/api.js` and the existing socket contract; this redesign is a
presentation layer over the same fields.

**Fields consumed** (from `status` / `status_history`) — names and nesting as the daemon
actually reports them:
`state` (`running` / `idle` / `paused` / `first run` / unreachable) · `pending_changes` ·
`status_history` (last 20) · `socket` · `cli`. The three counts — `conflicts`,
`destructive_actions`, `skipped_unsupported` — live inside the **nullable**
`last_plan_summary`, not at the top level, so a null summary means *unknown*, not zero.
The roots (`local_root` / `remote_root`) come from `response.config`, and the config keys are
`scan_interval_secs` and `events_driven` (**not** `event_driven_reconcile` — that key does not
exist in the engine).

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
