# System tray

**The only part of the app most people see most days.** Linux: GNOME with the AppIndicator
extension, or KDE Plasma's system tray. Tauri drives this through
libayatana-appindicator — see `gui/src-tauri/src/tray.rs`.

## What changed and why

Today it is a text menu where the label *is* the status report: `Sync now (3 pending)`,
`Resolve 1 conflict`, `Close window (keeps syncing in the tray)`. You have to read a list of
verbs to find out what's going on.

Now clicking the glyph opens the **compact panel from the main screen** — same hexagon, same seam,
same sentence — with the menu below it. You see the state; you don't parse it.

## The glyph — 16px, monochrome-safe

A tray icon may be forced monochrome and sits on a light or dark panel. So **state is carried by
fill, not hue.** Colour repeats the message where the desktop allows it; it never carries it alone.

Draw at `viewBox 0 0 120 120`, rendered `15–20px`, `stroke-width:9` (12 for the 13px inline
bullet). Ship symbolic SVGs; the theme recolours them.

| State | Construction | Mono | Colour |
| --- | --- | --- | --- |
| Up to date | `fill:none` + stroke | `#E8EBF0` | `#E8EBF0` |
| Syncing | track `#3E454E`/`#2A2E36` + animated segment `stroke-dasharray="70 230"`, `hexup 2.4s linear infinite` | `#E8EBF0` | warm gradient |
| Needs you | stroke + `circle cx=60 cy=60 r=17` filled | `#E8EBF0` | `#FF6B6B` |
| Paused | `stroke-dasharray="24 24"`, `opacity:.45` | `#E8EBF0` | `#E8EBF0` |
| Can't reach Proton | stroke + `M38 38 L82 82` | `#E8EBF0` | `#FF3B3B` |

Notes: the syncing dash is **70 230**, not the 62 238 used at large sizes — a longer segment is
needed for the motion to read at 16px. The needs-you form adds *mass* (a filled centre) rather than
a badge, so it's noticeable without being alarming. Paused interrupts the outline, which is
literally what's happening.

**Only five forms exist.** A solid filled hexagon is not a state — it was drawn that way by mistake
during design and corrected. Don't reintroduce it.

On a light panel the glyph inverts to `#14161A` with the same five forms. Nothing needs
re-specifying, because state is fill-based.

## The panel

The 360px compact panel from `02-shell.md`, with `border:1px solid rgba(255,255,255,.1)` (it
floats over the desktop, not over the app surface) and shadow `0 22px 54px rgba(0,0,0,.62)`.
Positioned `top:40px; right:16px` under the indicator on GNOME; bottom-right on KDE.

Below the panel body, a menu section: `border-top:1px solid #16181D; padding:6px`, rows
`padding:9px 13px; border-radius:8px; font-size:12.5px; color:#C9D0DA`, hovered/first row
`background:#101216`. Separator: `height:1px; background:#16181D; margin:5px 10px`.

### Menu contents by state

| State | Rows |
| --- | --- |
| Up to date | `Open Drive Sync` · `Sync now` · `Pause syncing` — sep — `Close window` · `Quit` |
| Syncing | `Open Drive Sync` · `Pause syncing` — sep — `Close window` · `Quit` |
| Needs you | (panel has `Review them`) `Open Drive Sync` · `Sync now` · `Pause syncing` — sep — `Close window` · `Quit` |
| Paused | `Resume syncing` · `Open Drive Sync` — sep — `Quit` |
| Can't reach | `Try again now` · `Open Drive Sync` — sep — `Quit` |

**Two labels carry a sub-label in 11.5px `#6D7783` and must keep it:**
- `Close window` — *keeps syncing*
- `Quit` — *stops syncing*

This is the single worst misunderstanding a tray app can cause, and the old build was right to
spell it out. Keep the sub-labels (shortened from "keeps syncing in the tray"), rendered as a
second baseline-aligned span at `gap:8px`.

### Panel copy by state

**Up to date** — `Up to date` 17px/600 / `2 minutes ago · 12,480 files` mono 11.5px.

**Syncing** — side labels, seam, hexagon with count, `Syncing 3 changes` 15px/600 (masked), then
two transfer rows.

**Needs you** — crimson outline hexagon with the count, `3 things need you`,
`One file changed on both sides.` / `Two deletions are waiting.` (two lines, centred), then
`Review them` as a full-width decision button.

**Can't reach Proton** — struck hexagon `#FF3B3B` + `Can't reach Proton Drive` +
`Nothing is lost. 4 changes are waiting and will go as soon as it's back.` +
`retrying in 40s · last reached 13:58` mono 11px. **Reassurance before the problem.**

**Paused** — dashed hexagon at `opacity:.55` with the two bars + `Paused` +
`7 changes have piled up since 13:20. Nothing will move until you resume.`

## In situ — the GNOME top bar

For reference when drawing mockups: `32px` bar, `rgba(8,9,11,.88)`. `Activities` at
`padding:0 14px` in 11.5px `#C9D0DA`; the clock centred via
`position:absolute; left:0; right:0; text-align:center` (11.5px/600 `#E8EBF0`); the status
cluster right at `gap:13px`, our indicator in a `padding:3px 5px; border-radius:6px;
background:rgba(255,255,255,.11)` chip to signal the open menu.

## Behaviour
- Left-click opens the panel; right-click opens the menu alone (KDE convention).
- The glyph updates from the daemon's status stream, not on a timer.
- Closing the window keeps the daemon running; `Quit` stops it. Confirm nothing on close; confirm
  nothing on quit either, but say what each does in the label.
