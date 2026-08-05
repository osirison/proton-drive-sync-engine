# Foundations

Everything here is theme-aware. Dark is the default; light values are in `12-light-theme.md`.
Add these to `gui/src/styles/tokens.css` alongside the existing variables.

## 1. Colour — dark theme

### Surfaces
| Token | Hex | Use |
| --- | --- | --- |
| `--surface` | `#0A0B0D` | Window background. Also the mask colour behind centred text crossing the seam. |
| `--panel` | `#0D0E11` | Standard panel/card |
| `--panel-alt` | `#0E0F12` | Secondary card (queued rows, inert list items) |
| `--panel-raised` | `#101216` | Active/selected row, in-progress transfer card, secondary button fill |
| `--track-inert` | `#191C21` | Progress-bar track; the un-travelled part of the hexagon |

### Lines
| Token | Hex | Use |
| --- | --- | --- |
| `--divider` | `#16181D` | Row separators, header/footer rules |
| `--border-subtle` | `#1A1D22` | Panel borders |
| `--border` | `#23262D` | Standard control borders, inputs, chips |
| `--border-strong` | `#2E323A` | Emphasised border; settled hexagon track |
| `--seam` | `#2A2E36` | The centre hairline (as a gradient — see §5) |

### Text — five tiers, use them in order
| Token | Hex | Size range | Use |
| --- | --- | --- | --- |
| `--text` | `#F2F4F7` | 15–42px | Headings, large numerals, primary values |
| `--text-bright` | `#E8EBF0` | 12–14px | Row primary text, filenames in lists, checkmark stroke |
| `--text-2` | `#C9D0DA` | 12–14px | Body with emphasis; menu items; consequence sentences |
| `--text-3` | `#99A2AE` | 12–14px | Standard body, descriptions |
| `--text-4` | `#828B98` | 12–13.5px | Quiet body, sub-descriptions |
| `--text-5` | `#6D7783` | 10.5–12px | Mono captions, metadata, timestamps (see AA note in README) |
| `--text-label` | `#626B78` | 9.5–10px | Uppercase section labels |
| `--text-disabled` | `#B9BEC6` light / `#4E5661` dark | — | Disabled control glyphs |

### Semantic colour — the contract
**Warm = leaving this computer.**
| Token | Hex | Use |
| --- | --- | --- |
| `--up-from` | `#E55B2B` | Gradient start |
| `--up-to` | `#FFB84D` | Gradient end |
| `--up-label` | `#FF9F1C` | "This computer" label, `→` arrows, upload accents |

Gradients are always `linear-gradient(90deg, #E55B2B, #FFB84D)` for horizontal bars, and
`x1=0 y1=0 x2=1 y2=1` for SVG strokes.

**Cool = arriving from Proton.**
| Token | Hex | Use |
| --- | --- | --- |
| `--down-from` | `#06B6D4` | Gradient start |
| `--down-to` | `#3B82F6` | Gradient end |
| `--down-label` | `#22D3EE` | "Proton Drive" label, `←` arrows, download accents |

SVG down-gradients are `x1=0 y1=1 x2=1 y2=0` (mirrored) so the two ramps read as opposing flows.

**Outlined crimson = a person must decide.** Never filled.
| Property | Value |
| --- | --- |
| Border | `rgba(255,107,107,.32)` — bands and cards |
| Background | `rgba(255,107,107,.05)` |
| Divider inside band | `rgba(255,107,107,.16)` |
| Text | `#FF9C9C` |
| Stroke (hexagon, dots) | `#FF6B6B` |
| Button | border `rgba(255,107,107,.4)`, bg `rgba(255,107,107,.1)`, text `#FF9C9C` |

**Solid red = irreversible, right now.** `#FF3B3B`. Used as: a filled 7px dot on a permanent
deletion; the hexagon stroke and warning glyph on the armed confirmation; the `Delete
permanently` button fill (with `#fff` text); the destructive row tint
`rgba(255,59,59,.05–.09)` and border `rgba(255,59,59,.38)`.

**Settled = no colour.** Neutrals only. This is a rule, not an absence — if a screen has nothing
to report it must contain no hue at all.

### Buttons
| Kind | Dark |
| --- | --- |
| Primary | bg `#F2F4F7`, text `#0A0B0D`, no border, weight 600 |
| Primary disabled | bg `#2A2E36`, text `#6D7783`, `cursor:default` |
| Secondary | bg `#101216` or `#16181D`, border `1px #23262D` or `#2E323A`, text `#C9D0DA`/`#E8EBF0` |
| Quiet | bg `transparent`, border `1px #23262D`, text `#99A2AE` |
| Decision | see outlined-crimson above |
| Destructive | bg `#FF3B3B`, text `#fff`, weight 600 |

**Rule: maximum contrast is the primary action.** In dark that is the white button; in light it is
the near-black one. This is why *Keep both* and *Keep it* are the brightest thing on their
screens — the safe choice is the loud one.

## 2. Typography

`Instrument Sans` for anything a person reads. `IBM Plex Mono` for anything the machine owns:
paths, globs, config keys, remote ids, byte counts, timestamps, daemon error strings, state names.
The mono is the signal that you are looking at ground truth.

`font-family:'Instrument Sans',system-ui,sans-serif`
`font-family:'IBM Plex Mono',ui-monospace,monospace`

Weights in use: 500, 600, 700 (sans); 400, 600 (mono). Nothing is 400-weight sans except long
body paragraphs.

### Scale — exact values, with their letter-spacing
| px | Weight | Letter-spacing | Use |
| --- | --- | --- | --- |
| 42 | 600 | `-0.035em`, `line-height:1` | Plan-preview side counts |
| 38 | 600 | `-0.035em`, `line-height:1` | Onboarding review counts |
| 34 | 600 | `-0.03em` | (reserved) |
| 32 | 600 | `-0.03em` | Main-screen headline |
| 30 | 600 | `-0.03em` | Onboarding step headlines, light-theme settled headline |
| 28 | 600 | `-0.025em` | Armed-deletion question, safe-plan verdict |
| 26 | 600 | `-0.025em` | Screen titles (Activity, Settings, Deletions, Conflicts) |
| 24 | 600 | `-0.025em` | Consent panel headline; hexagon numeral (mono 600) |
| 22 | 600 | `-0.025em` | Sub-headlines, first-sync headline |
| 20 | 600 | `-0.03em` | Numerals in stat positions |
| 18 | 600 | `-0.02em` | Panel titles in dialogs |
| 17 | 600 | — | Compact-panel headline, "Both sides agree" |
| 16 | 600 | — | Dialog titles, deletion filenames |
| 15 | 600 | — | Compact headline; mono 15 = the file under scrutiny |
| 14 | 600 | — | Card titles, setting names |
| 13.5 | 600 / 400 | — | Band titles (600); body under a headline (400) |
| 13 | 500 / 400 | `-0.01em` on titlebar | Titlebar product name (600), row text, buttons |
| 12.5 | 500 / 400 | — | Button labels, descriptions, menu items |
| 12 | 400 | — | Helper text, fine print |
| 11.5 | 400 | — | Mono metadata |
| 11 | 400 | — | Mono captions, key names, timestamps |
| 10.5 | 400 | — | Mono footnotes, menu sub-labels |
| 10 | 600 | `.16em`, uppercase | Section labels |
| 9.5 | 600 | `.14em`, uppercase | Compact-panel side labels, table headers |

`line-height`: `1` for large numerals; `1.05–1.1` for display headlines; `1.45–1.5` for
dense body; `1.55–1.6` for comfortable body; `1.65` for the longest explanatory paragraphs.
Set `text-wrap:pretty` on any paragraph over one line.

Uppercase labels are always `font-size:10px; font-weight:600; letter-spacing:.16em;
text-transform:uppercase` — never larger, never bolder.

## 3. Spacing and radii

**Window padding:** content `0 32px`; header `0 20px`; footer nav `0 40px`; footer action bar
`14px 32px`.

**Vertical rhythm inside content:** `16 / 18 / 20 / 22 / 24 / 26 / 30 / 34` px between blocks.
`margin-top:26px` is the standard gap from the title block to the first content block.

**Gaps:** `6` (tight lists), `8–10` (button rows, chips), `12–14` (card internals),
`16–20` (side-by-side halves), `26–36` (seam column padding: `padding-right/left:26–36px`).

**Always use flex/grid with `gap`.** No margin-based sibling spacing in lists or button rows.

**Radii:** `5` (tiny mono badges) · `6–7` (small controls, menu rows) · `8` (icon buttons,
small buttons) · `9` (buttons, inputs) · `10` (large buttons, list cards) · `11` (transfer
rows, inline bands) · `12` (window; radio cards) · `13` (content panels, choice buttons) ·
`14` (compact panel, emphasis cards) · `16` (chart panel, notification banner) ·
`99px` (pills, status chips) · `50%` (dots).

**Shadows.** Windows: `0 30px 70px rgba(0,0,0,.6)`. Compact panels and dialogs:
`0 24px 60px rgba(0,0,0,.6)`. Tray panel: `0 22px 54px rgba(0,0,0,.62)`. Notification banner:
`0 18px 44px rgba(0,0,0,.5)`. Light theme uses `.4–.45` alpha instead of `.6`.
No shadows anywhere inside a window — depth comes from surface steps.

## 4. Window geometry

The current app is a fixed `1040×764` Tauri window (`gui/src-tauri/tauri.conf.json`). Every
frame in the prototype is drawn at that size and **fits it exactly** — no internal scrolling
except where explicitly noted.

```
1040 × 764
├─ header            52px   flex:none
├─ content           flex:1 min-height:0
└─ footer nav      ~68px    flex:none
```

Some screens insert a footer action bar (`14px 32px`, `border-top:1px #16181D`) between
content and footer nav.

**Compact panel: 360px wide**, height content-driven. This is the tray panel and the
"check in without opening the app" surface. It repeats the main screen's hexagon, seam, headline
and transfer rows at reduced scale — same information, same order, ~1/8 the pixels.

**Resizing.** The design assumes the fixed window for now. When it becomes resizable: the seam
stays at 50%, the hexagon block stays vertically centred in its fixed-height hero, lists take the
flex, and below ~880px wide the two seam columns stack (local first) and the seam hairline is
dropped rather than rotated.

## 5. The seam — the core device

A 1px vertical hairline at `left:50%` separating **this computer (left)** from
**Proton Drive (right)**.

```css
/* dark */
background: linear-gradient(#0A0B0D, #2A2E36 26%, #2A2E36 74%, #0A0B0D);
/* light */
background: linear-gradient(#FAF8F5, #D9D5CE 26%, #D9D5CE 74%, #FAF8F5);
```

It fades in and out at both ends against the surface colour — it never touches an edge. The
percentage stops vary by block height (`10–30%` in, `70–90%` out); pick stops that put full
opacity across the content and fade over roughly the top and bottom eighth.

**Four hard rules, all learned the hard way:**

1. **It is drawn only when it means something** — when data is moving, or when a decision has two
   sides. A settled screen has no seam.
2. **It stops above any full-width band.** If an attention band, warning banner or footer bar
   spans the window, the seam terminates above it. Never overlap.
3. **Anything centred on the seam masks it.** Centred text and centred buttons that sit on the
   seam get `background:<surface>` plus `padding:0 14–18px` so the line passes *behind* them.
   `z-index` alone is not enough — the line would still show between glyphs.
4. **Direction is carried by position first, colour second.** A left-column row means "leaving"
   even in greyscale. Colour repeats the message; it never carries it alone.

## 6. The hexagon

Derived from `gui/src/assets/icon.svg` (`viewBox 0 0 128 128`, points
`64,10 110,36 110,92 64,118 18,92 18,36`) scaled by `120/128` onto a 120 viewBox.
**Pointy-top: vertex at top and bottom, flat vertical sides.** Getting this wrong (flat-top) is
the single easiest way to make the redesign look off-brand.

```
viewBox="0 0 120 120"
d="M60 9.4 L103.1 33.8 L103.1 86.3 L60 110.6 L16.9 86.3 L16.9 33.8 Z"
stroke-linejoin="round"
```

Perimeter ≈ 297 units — the number the dash arrays are tuned against.

**The outline itself is the animation track.** Nothing is nested inside it: no ring, no circle, no
inner arc. Two dash segments travel the hexagon's own edges in opposite directions.

### Sizes
`176/168` hero · `132` large hero · `116` first sync · `104` armed dialog · `96` empty
state · `88` safe plan · `80/76/74/72` compact and dialogs · `52/46/44` inline ·
`34` notification icon · `20/15/14` tray glyph · `13` inline bullet.

Stroke widths scale with size: `3.4` at 168px, `4.4–4.6` at 72–116px, `5–5.4` at 44–72px,
`6–7` at 34px, `9–12` at 13–20px. The mark should read as the same weight at every size.

### The five states
| State | Construction |
| --- | --- |
| **Settled** | Track `#2E323A` + check `M49 60 L57 68 L72 52` in `#E8EBF0`, `stroke-linecap:round` |
| **Syncing** | Inert track `#191C21`, then two full-path strokes with `stroke-dasharray="62 238"`, `stroke-linecap:round` — warm on `hexup`, cool on `hexdn` — plus the pending count as a centred mono numeral |
| **Needs you** | Single outline in the decision or destructive colour + centred mono numeral in the matching tint. No fill. |
| **Paused** | Track `#2A2E36` with `stroke-dasharray="14 12"`, `opacity:.55`, plus two bars: `rect x=51 y=49 w=5.5 h=22 rx=2.5` and `x=64`, fill `#99A2AE` |
| **Unreachable** | Outline `#FF3B3B` + strike `M40 40 L80 80` same width, `stroke-linecap:round` |

The warning variant (armed deletion, save refused) is the outline plus
`M60 38 L60 64` + `circle cx=60 cy=79 r=4.6`.

Centred numerals: `font-family:'IBM Plex Mono'; font-weight:600`, `text-anchor:middle`, and
`y` ≈ `68–74` depending on size — always optically centred, not mathematically.

## 7. Animation

```css
@keyframes hexup { from { stroke-dashoffset: 0 } to { stroke-dashoffset: -300 } }
@keyframes hexdn { from { stroke-dashoffset: 0 } to { stroke-dashoffset:  300 } }
@keyframes breathe { 0%,100% { opacity:.45; transform:scale(1) }
                      50%    { opacity:.8;  transform:scale(1.06) } }
@keyframes blip { 0%,100% { opacity:1;   transform:scale(1) }
                   50%    { opacity:.35; transform:scale(1.5) } }
```

| Where | Declaration |
| --- | --- |
| Hexagon, uploading | `hexup 3.2s linear infinite` |
| Hexagon, downloading | `hexdn 4.4s linear infinite`, `animation-delay:-2.2s` |
| Hexagon, dry run | `hexup 2.4s` / `hexdn 3.2s` with `dasharray 40 260` (faster, thinner — it's reading, not moving) |
| Tray glyph, syncing | `hexup 2.4s` with `dasharray 70 230` (longer segment so it reads at 16px) |
| Settled glow | `breathe 7s ease-in-out infinite` on a `480–520px` radial-gradient circle behind the hexagon: `radial-gradient(circle, rgba(232,235,240,.055), rgba(232,235,240,0) 68%)` |
| Status dot, syncing | `blip 1.6s ease-in-out infinite` |
| Text caret | `blip 1.1s ease-in-out infinite` on a `1.5×15px` bar |

The two hexagon durations are deliberately coprime-ish (3.2 / 4.4) with a negative delay so the
two segments rarely sit on the same edge — it should never look like one thick dash.

**Everything else is a transition, not an animation:** `140ms ease-out` for hover and press,
`220ms ease-out` for a panel or band appearing, `320ms ease-out` for the seam and its columns
fading in when traffic starts. Respect `prefers-reduced-motion`: drop the travelling segments to
a static 40%-opacity coloured outline, drop `breathe` and `blip` entirely, keep progress bars.

## 8. Symbols

The prototype uses Unicode glyphs sized to match the type. **Replace these with a real icon set
at build time** (a 1.5px-stroke line set — Lucide or Phosphor Light both fit); they are specified
here so the meaning survives the swap.

| Glyph | Meaning | Colour |
| --- | --- | --- |
| `→` | Leaving this computer | `#FF9F1C` (warm label) |
| `←` | Arriving from Proton | `#22D3EE` (cool label) |
| `⇄` | Keep both / two-way | inherits |
| `↷` | Moved or renamed to match | `#6D7783` |
| `＋` | Folder created | matches its side |
| `✕` | Deleted for good, or a failed pass | `#FF3B3B` / `#FF6B6B` |
| `⊘` | Skipped, never synced | `#6D7783` or `#FF9F1C` |
| `⌕` | Search | `#626B78` idle, `#99A2AE` active |
| `⋯` | Window menu | `#626B78` |
| `‹ ›` | Previous / next in a queue | `#4E5661` disabled, `#C9D0DA` active |
| `− ＋` | Steppers | `#C9D0DA` on `#101216` |
| `▲` | Warning | `#FF9F1C` or `#FF6B6B` |
| `✓` | Settled — **always the SVG path**, never a glyph | `#E8EBF0` |

Dots carry state at small sizes: filled `7–9px` circle = happened or irreversible; `2px` ring
= needs a decision; `#2E323A` filled = settled and inert.

## 9. Things to keep from the existing build

Not everything needs replacing. These are already right:

- **`journalctl --user -u proton-syncd`** as the escape hatch for history older than the last 20
  passes. Keep the wording "Open the system log".
- **The `.sync` directory is always ignored** and cannot be added to skip rules. Keep saying so.
- **Config writes are surgical** — only changed fields, comments and daemon-only keys preserved,
  and the write is rejected if the daemon's own parser would refuse the result. This is excellent
  behaviour and the redesign leans on it (see `08-settings.md`).
- **The delete gate** (type DELETE to arm) — kept, but narrowed to only appear when a plan
  actually deletes something.
- **Both-copies-kept conflict resolution** with the `.proton-cloud` suffix. The exact filename
  is quoted to users, so keep the suffix stable.
