# The shell

Every window shares this skeleton. Build it once.

## Header — 52px, `flex:none`

`padding:0 20px`, `display:flex; align-items:center; gap:12px`. **No bottom border** — the
header floats on the surface. (The old build had a `1px` rule and a filled `#101a2c` bar; both
are gone.)

| Slot | Spec |
| --- | --- |
| App mark | `icon.svg` at `20×20`. `opacity:.65–.75` on settled/secondary screens, `1` when something is happening. |
| Product name | `Proton Drive Sync` — 13px / 600 / `-0.01em`. `#14161A`…`#F2F4F7` when active, `#99A2AE` when the screen is settled. |
| Spacer | `flex:1` |
| Status chip | See below. Omitted on onboarding (replaced by `step 1 of 2` in mono 11px `#6D7783`). |
| Menu button | `30×30`, `border-radius:8px`, `border:none`, `background:transparent`, glyph `⋯` 15px `#626B78` |

### Status chip
`padding:5px 12px; border-radius:99px`, mono 11px, with a `6px` dot at `gap:7px`.

| State | Border | Text | Dot |
| --- | --- | --- | --- |
| idle | none | `#626B78` | `#2E323A` |
| syncing | `1px #23262D` | `#99A2AE` | `#FFB84D` + `blip 1.6s` |
| n waiting (decisions) | `1px rgba(255,107,107,.35)` | `#FF9C9C` | `2px` ring `#FF6B6B` |
| n waiting (deletions) | `1px rgba(255,107,107,.35)` | `#FF9C9C` | filled `#FF3B3B` |
| rehearsal | `1px #23262D` | `#99A2AE` | none — text only: `rehearsal · nothing has changed` |

The old titlebar also carried the folder pair as a mono string and a `◐ Theme` button. The folder
pair moves to the footer or the seam labels; the theme toggle moves into the `⋯` menu.

## Footer navigation — the four doors

`padding:0 40px 18–22px`. Inside: `display:flex; align-items:center; justify-content:center;
gap:34px; padding-top:14–20px; border-top:1px solid #16181D`.

```
Activity     Plan a sync     Settings     Details
```

13px, `#828B98` (dark) / `#4B5563` (light); the active one is `#F2F4F7` / `#14161A`.
**These four never move and never change order**, on any screen, in any state. They replace the
old 214px left sidebar entirely.

Optional line beneath, centred, mono 11px `#626B78`: the folder pair, or today's byte totals, or
nothing. Never anything you need.

The old sidebar's badge counts are not reproduced here — a decision waiting is announced by the
status chip and by an attention band on the main screen, not by a number in navigation.

## Footer action bar

Screens that commit something (Settings, Plan, Deletions) insert this between content and footer
nav: `padding:14px 32px; border-top:1px solid #16181D; display:flex; align-items:center; gap:12px`.

Order is always: **consequence text on the left** (12px `#6D7783`, or `#FF9F1C` when the
pending change has a cost), `flex:1` spacer, then secondary action, then primary. The primary is
disabled (`#2A2E36` / `#6D7783`) until the screen's gate is satisfied.

## Content area

`flex:1; min-height:0; padding:0 32px; margin-top:22–26px`.

**Set `overflow:hidden` or size the content to fit.** `overflow:visible` on a `flex:1` child
lets content paint over the footer — a real bug found twice during this design. Where a list can
genuinely exceed the space, use `overflow-y:auto` so the clip reads as a scroll region, and make
sure the cut falls on a row boundary rather than through a group header.

## The 360px compact panel

Used as the tray panel and the quick check-in. `width:360px`, `border:1px solid #23262D`,
`border-radius:14px`, `overflow:hidden`, shadow `0 22px 54px rgba(0,0,0,.62)`.

Structure, top to bottom:
1. **Hero** — `padding:22–30px 20–22px 14–24px`, `flex-direction:column; align-items:center`.
   Seam labels (9.5px/600/`.14em` uppercase, `This computer` left, `Proton` right, space-between)
   only when syncing. Hexagon `72px`. Headline 15–17px/600 with the seam mask if the seam is drawn.
   Sub-line mono 11.5px `#6D7783`.
2. **Transfer rows** — `padding:0 14px 12px`, `gap:6px`. Cards `border-radius:9px`,
   `padding:9px 11px`, mono 11.5px filename, direction arrow, 2px progress bar pinned to the
   bottom edge.
3. **Attention band** — when needed: `margin:12px 14px`, decision colours, `border-radius:11px`,
   `padding:11px 13px`, ring dot + 12.5px/500 text + `›`.
4. **Footer** — `border-top:1px solid #16181D; padding:10px 16px`, mono 10.5px `#6D7783` status
   on the left, then two small buttons (`padding:5px 11px; border-radius:7px; font-size:11.5px`).

When it lives in the tray it gains a menu section instead of the two buttons — see `10-tray.md`.
