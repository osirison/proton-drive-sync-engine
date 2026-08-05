# Plan a sync — the rehearsal

**Route:** footer nav → *Plan a sync*. **Purpose:** know whether it is safe to run.

## What changed and why

The old screen opened with **ten identical stat tiles** — `total`, `uploads`, `downloads`,
`remote_directories_created`, `local_directories_created`, `local_moves`, `remote_moves`,
`auto_links`, `conflicts`, `type_conflicts` — of which seven read zero, then a raw action table
with `action` / `path` / `entity` / `remote_id` columns. It answered "what are the counts"
when the only question is "is this safe to run".

Now it opens with a sentence and a verdict. The seam does the counting. Anything destructive breaks
out of the seam into its own band with an escape hatch. **Nothing that reads zero gets a tile** —
an absent category is simply absent, so a simple plan produces a short screen.

## State A — a plan that would destroy something

``@
header 52  (status chip = "rehearsal · nothing has changed")
title row + "Check again"
seam block      (two side counts)
destructive band
flex:1 action list
footer action bar (gate + two buttons)
``@

**Title:** `The next sync moves 9 things` (26px/600) /
`One of them can't be undone. Everything here is a rehearsal — nothing has changed yet.`
Right: `Check again` quiet button.

**Seam block** `margin-top:24px`, seam `top:8px; bottom:0`, grid `1fr 1fr`:
- Left: `Leaving this computer` label → `3` at **42px/600/`-0.035em`/`line-height:1`** +
  `files, 4.1 MB` 14px `#99A2AE` → `Plus one new folder created on Proton Drive to hold them.`
  12.5px `#828B98`.
- Right, right-aligned: `Arriving from Proton` → `2` + `files, 2.6 MB` →
  `One file you renamed will be renamed here to match.`

**Destructive band** `margin-top:22px`: `border:1px solid rgba(255,59,59,.38);
background:rgba(255,59,59,.06); border-radius:14px; padding:16px 20px; gap:16px` —
`34px` warning hexagon, then `One file gets deleted for good` (14px/600 `#FF9C9C`) and
`**archive/old-notes.md** is removed from Proton Drive. It's already gone from this computer, so nothing will bring it back.`
(12.5px `#C9D0DA`, path in mono 12px), then `Leave it alone`
(`#101216`/`#2E323A`/`#E8EBF0`/600). **The dangerous thing is never just a row in a list.**

**Action list** — `Every action, in order` label, right-aligned mono 11px
`9 actions · 1 conflict kept as both copies`. Rows `padding:8px 2px;
border-bottom:1px solid #16181D; gap:13px`:

`glyph (13px, centred) · path (mono 12.5px, flex:1, ellipsis) · plain-English outcome (12px #828B98)`

| Glyph | Path | Outcome | Row tint |
| --- | --- | --- | --- |
| `✕` `#FF3B3B` | `archive/old-notes.md` | `deleted for good on Proton` `#FF9C9C` | `rgba(255,59,59,.05)` |
| `→` `#FF9F1C` | `docs/spec.md` | `sent to Proton` | — |
| `＋` `#FF9F1C` | `photos/trip` | `folder created on Proton` | — |
| `←` `#22D3EE` | `reports/q3-summary.pdf` | `brought to this computer` | — |
| `↷` `#6D7783` | `notes/old.md → notes/archive/old.md` | `moved to match Proton` | — |
| ring dot | `notes/todo.txt` | `both copies kept, nothing lost` | — |

No `entity` column, no `remote_id` column. Those live behind *Details*.

**Footer action bar:** a `190px` input, placeholder `type DELETE to allow it`, beside
`Only needed because this plan deletes something.` (12px `#6D7783`, `max-width:210px`).
Then `Run it without the deletion` (quiet) and `Run this sync` (primary, disabled until the gate
passes). **The second path matters** — you shouldn't have to choose between all of it and none.

## State B — the ordinary safe plan

The screen shrinks. Hero block `height:300px` centred, seam `top:24px; bottom:-40px`:
`88px` settled hexagon, `Nothing gets deleted` 28px/600 (masked), and
`Five files move, both sides end up with everything. This plan is safe to run.` 13.5px
`#828B98` `max-width:460px` (masked).

Then the same seam block, but each side lists its files inline: rows of
`glyph · path · size` separated by `border-top:1px solid #16181D`, `padding:7px 0`. New folders
show `new folder` instead of a size, in 11.5px `#6D7783` with the path in `#99A2AE`.

No destructive band. No gate. Footer: `Checked 40 seconds ago against both sides.` +
`Check again` + `Run this sync` (primary, enabled).

## State C — working it out

520px window. Seam `top:60px; bottom:60px`. `104px` hexagon running `hexup 2.4s` /
`hexdn 3.2s` with `dasharray 40 260` and **no numeral** — it's reading, not moving.
`Working out what would change` 22px/600 (masked),
`Comparing both sides. Nothing is being touched.` 13px (masked),
`8,431 of 12,480 files` mono 11.5px (masked), `Stop` quiet button —
**`background:#0A0B0D`, not transparent**, so the seam passes behind it.

## Behaviour
- Runs `proton-syncd --dry-run`; index is read-only. Say `rehearsal · nothing has changed` in the
  chip the whole time.
- `Run it without the deletion` applies the plan minus destructive actions — needs daemon support
  for a filtered apply; if unavailable, hide the button rather than faking it.
- Re-check invalidates the plan; if the plan changes while the gate is armed, clear the input.
