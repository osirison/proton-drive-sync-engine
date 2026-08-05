# Conflicts — "You both changed this file"

**Route:** from the attention band, the status chip, or a conflict notification.
**Purpose:** make one decision, safely.

## What changed and why

The old screen was a 236px list of conflicts beside a split diff, with four equal-weight buttons
(`Keep mine` / `Use Proton's` / `Keep both` / `Decide later`) and a line-numbered diff as the
main content. Two problems: a list of conflicts is a list of postponed decisions, and the diff
answers "what differs" when the question is "which do I want".

Now: **one conflict fills the window**, the file sits *on* the seam, and the diff is one click
away. The remaining queue is a small list at the bottom of the diff view, not a permanent sidebar.

## Layout

```
header 52
title row        (26px title + 13.5px sub, and 1 of 3 + ‹ › on the right)
flex:1  content  — seam top:56 bottom:8
  hexagon 44 + filename + meta      (centred, masked, z-index:1)
  two version cards                 (grid 1fr 1fr, padding-right/left:30px)
  "See the exact differences"       (centred, masked)
three choice buttons + note
footer nav
```

### Title row
`You both changed this file` — 26px/600/`-0.025em`.
`Nothing has been lost — both versions are still here, and syncing carries on around this.` —
13.5px `#828B98`, `margin-top:7px`.

Right side: `1 of 3` mono 11.5px `#828B98`, then `‹` (disabled: `#4E5661`, `cursor:default`)
and `›` (`#101216` fill, `#C9D0DA`), both `30×30`, `border-radius:8px`,
`border:1px solid #23262D`.

### The file, on the seam
Hexagon `44px`, decision outline `#FF6B6B` `stroke-width:5`, numeral `3` mono 30px/600
`#FF9C9C` at `y=72`, `fill:#0A0B0D` so the seam doesn't show through.

Below, both masked: path in **mono 15px `#F2F4F7`** (`notes/todo.txt`) and
`a plain text file · last agreed 3 hours ago` in mono 11.5px `#6D7783`.

### Version cards
`border:1px solid #23262D; border-radius:13px; background:#101216; padding:16px 18px`,
`margin-top:11px` under a 10px uppercase label.

Left label `Your version · this computer` `#FF9F1C`.
Right label `Proton's version · from another device` `#22D3EE`, `text-align:right`.

Card contents:
1. **What happened**, 14px/600 — `You added a line, 5 minutes ago` /
   `Changed a line and added one, 2 minutes ago`.
2. **What differs, in words**, 13px `#99A2AE` `line-height:1.55` —
   `Yours has buy milk where Proton's has something else, and is otherwise the same.` The
   quoted content is mono 12px `#E8EBF0` inline.
3. **Metadata row** — `margin-top:14px; padding-top:13px; border-top:1px solid #1A1D22; gap:14px`,
   three mono 11px `#6D7783` items: bytes, line count, edit time.

Generating (2) needs a diff summary from the daemon, not just a byte diff. If that isn't
available, fall back to the metadata row alone — **do not** fall back to showing the raw diff
here; that's what the disclosure is for.

### The three choices
`display:grid; grid-template-columns:1fr 1fr 1fr; gap:12px`, each
`text-align:left; padding:15px 17px; border-radius:13px`.

| Button | Style | Copy |
| --- | --- | --- |
| Keep mine | `border:1px solid rgba(255,107,107,.4)`, `background:rgba(255,107,107,.07)`, title `#FF9C9C` | `→` + `Keep mine` / `Your version goes to Proton Drive. Proton's version is discarded.` |
| **Keep both** | `border:1px solid #2E323A`, `background:#F2F4F7`, title `#0A0B0D`, body `#3E454E` | `⇄` + `Keep both` / `Nothing is lost. Proton's copy lands beside yours as todo.proton-cloud.txt.` |
| Use Proton's | same as Keep mine | `←` + `Use Proton's` / `Proton's version replaces the file on this computer. Yours is discarded.` |

**This is the most important detail on the screen.** Keep both is the primary (maximum-contrast)
button because nothing is lost. Both discarding options wear the decision outline. The arrow
glyphs keep their side colours even though the titles are crimson.

Below: `Discarding a version can't be undone from here.` 12px `#6D7783`, and `Decide later`
as a quiet button on the right.

## Diff view (disclosure)

Replaces the version cards. Header compresses to a `34px` hexagon + mono 15px path +
`Two lines differ. Everything else in the file matches.` (12.5px `#828B98`).

Diff panel: `border:1px solid #1A1D22; border-radius:13px; background:#0D0E11`,
`display:grid; grid-template-columns:1fr 1px 1fr` with the centre cell `background:#2A2E36` —
**the seam becomes the diff's gutter**. `padding:12px 0` each side.

Line: `display:flex; gap:12px; padding:3px 16px` — number mono 11px `#4E5661` in a `16px`
right-aligned span, then content mono 12.5px. Unchanged `#99A2AE`; changed `#F2F4F7` on
`rgba(229,91,43,.14)` (left) or `rgba(6,182,212,.14)` (right) with the number in the side
colour. Absent lines: number `·` in `#2E323A`, text `not in your version` `#3E454E` — the
two sides stay row-aligned.

Under it: `2 lines differ · 3 lines identical` mono 11px, then `Open both in an editor` and
`Hide differences` as quiet buttons.

Then the remaining queue: `Still waiting after this one` label, rows of
`ring dot · path · reason · n of 3` — `design/logo.svg` / `both changed it`,
`photos/trip` / `a folder here, a file there`.

## Cleared state
520px-wide window: `96px` settled hexagon, `Nothing left to decide` 22px/600,
`You settled 3 files. Two kept both versions, one took Proton's copy.` 13px `#828B98` centred
`max-width:300px`, `Back to sync` quiet button.

## Behaviour
- Choosing advances to the next conflict with a 220ms crossfade; the header stays put.
- `Decide later` skips without resolving; the item stays in the queue.
- Type conflicts (a folder one side, a file the other) use the same frame with different card
  copy — there is no diff to show, so the disclosure is hidden.
- Non-text conflicts (images, PDFs) replace the diff with a side-by-side preview; the metadata row
  and the three choices are unchanged.
