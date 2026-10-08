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

With **two folders or more** the table gains a variant of its own — see "Two folders or more" below.
The rows above are what one folder draws, and what a daemon that lists no folders draws, exactly.

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

## Two folders or more

Nothing here draws below two folders: a person with one folder sees every glyph, title, panel and
row above, unchanged (DEVIATIONS §107, rule R-N1). What follows is what two or more add.

**Every folder has its own pause.** There is no `Pause all` row and no state behind one, on any
surface; the tray, the window and the `proton-sync` command line pause and resume each folder
separately. The one `Pause syncing` row of the table above becomes one row per folder.

### The glyph is the worst folder's state

One glyph, from the five forms (no sixth), showing the **worst** state any folder is in. Highest
first, with why each sits where it does:

| Rank | State | Why here |
| --- | --- | --- |
| 1 | Can't reach (the daemon) | the control socket; there are no per-folder answers to rank |
| 2 | Signed out | the session is per user, so every unpaused folder says it at once |
| 3 | Last sync failed | one folder's pass failed, or its folder is not there; never hidden behind a healthy one |
| 4 | Never synced | only from a full reply, so only for the folder the reply is about |
| 5 | Syncing | something is moving; a folder you paused does not outrank it |
| 6 | Paused | above up to date: an all-clear glyph over a folder that is not syncing is the lie to avoid; below syncing: you did it on purpose, and the title says which |
| 7 | Up to date | only when every folder is |

So: one folder paused and the other up to date shows the paused glyph; one paused and the other
syncing shows the syncing glyph; one failed and anything else shows the can't-reach glyph — which means
"this folder's last sync failed", not "Proton is out of reach", and **the title is where the
difference is said**.

### The title says what the glyph cannot

When the folders are not all in the glyph's state, the title names them, worst first: `Proton Drive
Sync — documents paused, photos up to date`. At most three are named, then `+n more`, and the whole
title stays under 140 characters: a name that does not fit is cut with `…`, the state beside it never
is (a folder may be named with 64 characters). When they all are in the glyph's state, it is the
one-folder title.

### The panel is the worst folder's own

The panel's hero is that folder's own, headline unchanged, preceded by **its name as one mono line**
(11.5px `#6D7783`, centred, one line, ellipsis) — `documents` above `Paused`. Panel borders and hero
padding are the one-folder panel's, and so is the sub-line, with one exception: a paused hero names the
folder in its second sentence — `7 changes have piled up since 13:20. Nothing in documents will move
until you resume.` — because another folder may be syncing under it, and "nothing will move" would be
untrue of the app.

What the panel counts for `Needs you` is what it can see: **the withheld deletions of every folder**
(each folder's summary carries its own count), and conflicts only for the folder it scans (a disk walk
the tray does not run for the others) — the limit is stated here, not filled. A deletion waiting in a
folder the panel is not about is therefore never hidden behind `Up to date` or `Paused`:

- The panel's folder is the worst by rank, **except that a folder that is up to date and has a decision
  waiting outranks one that is merely paused, or up to date with nothing to decide.** A pause is
  something you did; a decision is the one thing the tray cannot do for you. What is moving or wrong —
  syncing, a failed sync, a signed-out session, a stopped daemon — still outranks a decision, as it does
  at one folder. The glyph and the title are not touched by this: they follow the rank table alone.
- `Review them` names the folder it is drawn for, and opens the window with **that folder selected**.
  At one folder it opens the window and nothing else.

### The menu

```
Open Drive Sync
Sync now                       only while some UNPAUSED folder is idle
-----
Pause documents                Resume documents, while that folder is paused
Pause photos
-----
Close window   keeps syncing   only while some folder is unpaused
Quit           stops syncing
```

- **One row per folder, worst first**, ties in the order the daemon lists them. `Pause {name}` or
  `Resume {name}` by that folder's own state, so a label never offers what the folder is not in a state
  to take.
- **`Sync now`** is one row, unchanged, that asks every unpaused folder to sync. It is chosen from the
  folder list and not from the glyph's state: while one folder syncs the glyph says syncing, and the
  folders that are idle still want it. It is absent only when every unpaused folder is already syncing.
- **The panel draws pause rows for five folders**, worst first, then one row — `2 more folders` — that
  does what `Open Drive Sync` does. The native menus draw every folder.
- The set that is `Can't reach` while the daemon answers (`Try again now`) gains the same group
  between rules. An expired session, a folder that has never synced and a stopped daemon keep their
  sets: there is nothing to pause, or nobody to send it to.
- A row's action names its folder, and a click on a menu drawn earlier acts on **the folder its label
  named**, or on nothing if that folder is gone — never on whichever folder stands in that place now.
  The name is written on the request **for the default folder too**: the default is addressed by
  omission everywhere else, and a click on a stale `Pause documents`, after a restart changed which
  folder stands first, would otherwise pause the new default.

The frames: `10a Two folders`, `10a Two folders paused`, `10a Two folders failed`, `10a Many folders`.

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
