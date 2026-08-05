# Main screen

**Route:** default view. **Purpose:** answer "is everything safe?" in under a second, and show
what's moving if anything is.

## What changed and why

The old Overview packed in a hexagon card, a "Needs you" card, four stat tiles
(`pending_changes`, `conflicts`, `destructive_actions`, `skipped_unsupported`), a
seven-filter activity ledger, a permanent orange deletion warning banner, a 214px sidebar and a
daemon status bar. Nine competing regions, most of them showing zero.

Now: one hexagon, one sentence, and the seam — which draws itself in only when there is traffic.
The activity ledger is gone from this screen entirely (it was a table of things you already knew
about, occupying the most valuable space); it became `07-activity.md`. The stat tiles are gone;
their raw fields live behind *Details*. The permanent warning banner is gone — a warning that is
always on screen is furniture, and the consequence copy moved to the moment of action.

## State A — settled

``@
header 52
hero   394  (fixed height — this is what keeps the hexagon from moving)
flex:1      (empty)
footer nav
``@

Hero: `position:relative; display:flex; flex-direction:column; align-items:center;
justify-content:center`.

| Element | Spec |
| --- | --- |
| Glow | `position:absolute`, `480×480`, `border-radius:50%`, `radial-gradient(circle, rgba(232,235,240,.05), rgba(232,235,240,0) 68%)`, `breathe 7s ease-in-out infinite`, `pointer-events:none` |
| Hexagon | `168px`, settled state, `stroke-width:3.4`, track `#2E323A`, check `#E8EBF0` `stroke-width:3.6` |
| Headline | `Everything is up to date` — 32px/600/`-0.03em`, `#F2F4F7`, `margin-top:30px` |
| Sub-line | `last synced 2 minutes ago · 12,480 files · 41.2 GB` — mono 13px `#828B98`, `margin-top:12px` |
| Buttons | `margin-top:28px`, `gap:10px`: `Sync now` (secondary, `#101216`/`#23262D`… on light: white fill) and `Pause` (quiet) — both `padding:11px 22px; border-radius:10px; font-size:13.5px` |

Footer nav plus a centred mono 11px `#626B78` line: `~/ProtonDrive ⇄ /Drive/RemoteFolder`.

**No seam. No colour anywhere.** This is the rule made visible.

## State B — syncing

Identical skeleton; the hexagon does not move. Added:

| Element | Spec |
| --- | --- |
| Seam | `position:absolute; top:0; bottom:-114..-150px; left:50%` — extends below the hero into the transfer columns. Stops above any attention band. |
| Side labels | `position:absolute; top:16px`, `left:32px` / `right:32px`. Label 10px/600/`.16em` uppercase — `This computer` `#FF9F1C`, `Proton Drive` `#22D3EE`. Path beneath in mono 12px `#828B98`, `margin-top:6px`. |
| Hexagon | `168px` syncing state; centred mono numeral = pending count, 21px/600 `#F2F4F7` |
| Headline | `Syncing 3 changes` — 32px/600, **with `background:#0A0B0D; padding:0 18px`** (seam mask) |
| Sub-line | `started 14 seconds ago · 2 leaving, 1 arriving` — mono 13px `#828B98`, masked, `padding:2px 16px` |
| Button | `Pause` only — `Sync now` is meaningless mid-sync |
| Transfer columns | `flex:1` grid `1fr 1fr`, `padding:0 32px; margin-top:34px; align-content:start`. Left column `padding-right:26px`, right `padding-left:26px` |

### Transfer row
`border:1px solid #23262D; border-radius:11px; background:#101216; padding:11px 13px;
position:relative; overflow:hidden`.

Left column: filename (mono 12px `#E8EBF0`, `flex:1`, ellipsis) → size (mono 11px `#6D7783`)
→ `→` `#FF9F1C`. Right column: `←` `#22D3EE` first, then filename, then size. **The arrow is
on the outside edge in both columns**, pointing away from the seam.

Progress: `position:absolute; left:0; right:0; bottom:0; height:2px; background:#191C21` with an
inner div at the real percentage, filled with that side's 90deg gradient.

Queued rows: `border:1px solid #1A1D22; background:#0E0F12`, filename `#99A2AE`,
`queued` in mono 11px `#6D7783`, arrow `#6D7783`, no progress bar.

Footer line becomes `386 MB sent · 1.1 GB received today`.

## State C — a decision is waiting

Syncing continues underneath — this is additive, never a replacement. Between the transfer columns
and the footer nav:

`border:1px solid rgba(255,107,107,.32); background:rgba(255,107,107,.05); border-radius:14px;
overflow:hidden`, in a `padding:0 32px 18px` wrapper. One row per category, separated by
`border-bottom:1px solid rgba(255,107,107,.16)`.

Row: `padding:14px 18px; gap:14px` — dot (7px; `2px` ring `#FF6B6B` for conflicts, filled
`#FF3B3B` for permanent deletions) → title 13.5px/600 → sub-line mono 11.5px `#828B98`
`margin-top:3px` → action button (decision style, `padding:8px 15px; border-radius:8px;
font-size:12.5px`).

Exact copy:
- `One file changed on both sides` / `notes/todo.txt · both copies kept, nothing lost` / `Compare`
- `Two deletions are waiting on you` / `1 removes from this computer permanently · 1 goes to Proton's Trash` / `Review`

Status chip becomes `3 waiting`. Headline stays `Syncing 3 changes`; the sub-line becomes
`3 other changes are waiting on you`. **The count in the hexagon is transfers, not decisions** —
the decisions are counted in the chip and the band.

## Compact versions
All three states exist at 360px (see `02-shell.md`). Settled: hexagon + `Up to date` +
`2 minutes ago`. Syncing: side labels + seam + hexagon + two transfer rows. Needs-you: crimson
outline hexagon with the numeral, `3 things need you`, two-line explanation,
`Review them` full-width button, and `syncing continues` / `Later` in the footer.

## Behaviour
- `Sync now` → triggers a pass; state B within ~1s.
- `Pause` → paused state; hexagon dashed at `opacity:.55`; headline `Paused`; sub-line counts
  what has piled up; button becomes `Resume`.
- Seam, side labels and transfer columns **fade in over 320ms ease-out** when a pass starts and
  fade out when it ends. The hexagon crossfades between states over 220ms; it never moves or
  rescales.
- Transfer rows appear in flight order, cap at ~6 visible with `+n more` in mono if exceeded.
