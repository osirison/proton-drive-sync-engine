# Light theme

The existing app has a theme toggle. The rule for light is the important part.

## Light is not an inversion

The dark accents are **luminous** — amber and sky work because they glow against near-black. Put the
same values on white and they read as pastel and stop meaning anything. So light uses the
**darker end of each ramp**. Same three meanings, re-pitched.

Two things transfer unchanged:
- **Maximum contrast is the primary action.** White button on dark → near-black button on light. So
  `Keep both` and `Keep it` stay the loudest thing on their screens.
- **No colour still means settled.** A light screen at rest is warm off-white and one grey hexagon —
  arguably calmer than the dark version.

The surface is **`#FAF8F5`, not `#fff`**. Pure white makes burnt orange look muddy and leaves
nowhere for a raised card to go. Cards *are* white, so depth reads as light rather than shadow.

## The mapping

| Meaning | Dark | Light |
| --- | --- | --- |
| Leaving this computer | `#E55B2B → #FFB84D` | `#B23F14 → #D97706` |
| — label | `#FF9F1C` | `#B23F14` |
| Arriving from Proton | `#06B6D4 → #3B82F6` | `#0E7490 → #1D4ED8` |
| — label | `#22D3EE` | `#0E7490` |
| A person must decide | `#FF6B6B` outlined | `#BE123C` outlined |
| — border / bg / divider | `.32 / .05 / .16` | `rgba(190,18,60,.28–.35) / .03–.06 / .14` |
| Irreversible, right now | `#FF3B3B` | `#DC2626` (dark text `#B91C1C`) |
| — border / bg | `.38 / .06` | `rgba(220,38,38,.4) / .04` |
| Settled hexagon track | `#2E323A` | `#C7CBD2` |
| Syncing hexagon track | `#191C21` | `#E4E1DB` |
| Surface | `#0A0B0D` | `#FAF8F5` |
| Panel / card | `#0D0E11` / `#101216` | `#FFFFFF` / `#FDFCFA` |
| Divider | `#16181D` | `#EDEAE5` |
| Border subtle / std / strong | `#1A1D22` / `#23262D` / `#2E323A` | `#EDEAE5` / `#E6E3DE` / `#E0DCD5`, `#D6D2CB` |
| Seam | `#2A2E36` | `#D9D5CE` |
| Progress track | `#191C21` | `#EDEAE5` |
| Text primary | `#F2F4F7` | `#14161A` |
| Text body | `#C9D0DA` / `#99A2AE` | `#374151` / `#4B5563` |
| Text quiet | `#828B98` / `#6D7783` | `#4B5563` / `#6B7280` |
| Text disabled | `#4E5661` | `#B9BEC6`, `#9CA3AF` |
| Primary button | `#F2F4F7` bg / `#0A0B0D` text | `#14161A` bg / `#FAF8F5` text |
| Secondary button | `#101216` bg / `#23262D` border | `#FFFFFF` bg / `#D6D2CB` border |
| Disabled destructive | `#8A5A5A` | `#C08A8A` |
| Settled glow | `rgba(232,235,240,.055)` | `rgba(20,22,26,.045)` |
| Window shadow | `rgba(0,0,0,.6)` | `rgba(0,0,0,.4–.45)` |

> **Two rows above are ambiguous — do not apply them mechanically without reading
> `IMPLEMENTATION-PLAN.md` §1.3, conflicts 8 and 9.** *Border subtle / std / strong* lists four
> values for three tokens (`#D6D2CB` is the secondary-button border and has its own row below, so
> the mapping is three-to-three; which of `#E6E3DE` / `#E0DCD5` is std versus strong still needs
> per-surface measurement). *Text disabled* gives two light values for one dark token — the frames
> use `#B9BEC6` on disabled glyphs and captions, `#9CA3AF` once on plain body text, so two light
> disabled tiers exist but are unnamed. Both are resolved by measurement in P0.2, and this table
> is propagated to the seven screens with no light frame, so an error here does not get caught
> later.

**Everything else is identical** — geometry, type, spacing, radii, animation, symbols, copy. The
seam mask colour changes from `#0A0B0D` to `#FAF8F5`; that's the only structural edit.

## Text-on-tint

Consequence sentences inside destructive/decision cards move from `#C9D0DA` to `#374151`, and
the emphasised loss from `#FF9C9C` to `#B91C1C`. Metadata inside those cards moves from
`#828B98` to `#4B5563` — on a light tint the quiet tier is too quiet.

## Frames drawn

Main screen (settled, syncing), Deletions, Conflicts, and the three compact panels, plus the tray
glyph on a light panel. **The other seven screens are not drawn** — apply the table above
mechanically; there are no light-specific layout decisions.

## Implementation

The existing `tokens.css` already has a light block and a `◐ Theme` toggle in the titlebar. Keep
that mechanism, move the toggle into the `⋯` menu, extend both blocks with these values, and add a
`prefers-color-scheme` default with the explicit choice persisted. **The SVG gradients need
theme-aware stops** — either duplicate the `<linearGradient>` defs per theme or drive the stops
from `currentColor`/CSS variables.
