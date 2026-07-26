# Handoff: Proton Drive Sync — Linux desktop UI

## Overview

A desktop UI for `proton-drive-sync-engine` — an existing Rust two-way sync engine
for Proton Drive that ships as a systemd user daemon (`proton-syncd`) and a CLI
(`proton-sync`). The engine works; it has no graphical interface. This handoff
covers the client that gives it one: status and health, activity history, conflict
resolution, dry-run review before destructive passes, settings, onboarding, and a
tray indicator.

Audience is prosumer Linux users on Fedora, Ubuntu and Arch — people who will read
a plan before applying it. The UI therefore exposes the engine's real vocabulary
(field names, globs, paths, remote ids) instead of hiding it.

## About the design files

The files in this bundle are **design references created in HTML** — prototypes of
intended look and behaviour, not production code to copy. The task is to recreate
them in the target environment using its established patterns: an Electron or
Tauri shell can follow them closely (they were built as web UI per the brief); a
GTK4/libadwaita or Qt implementation should treat them as a specification and use
native widgets.

`Proton Sync UI.dc.html` is a single self-contained page holding every option, laid
out as a canvas. Open it in a browser and pan/zoom. Options are labelled with
stable ids (`1a`, `2a`, `5a`…) and this README refers to them by id.

**Start with option 5a** — the complete app shell. Everything else is either a
component study or an alternative that was explored and set aside.

## Fidelity

**High fidelity.** Colours, type, spacing, radii, copy and interaction behaviour
are final and specified exactly (see Design tokens below, and section 7 of the
companion document). Layout structure and state behaviour are final. Not
specified: the GUI toolkit, and the six items listed under "Engine gaps".

## The companion document

`Proton Sync UI Handoff.dc.html` in this bundle is the **implementation contract**:
for every dynamic element on every screen it names the socket command, JSON field,
or file that supplies the value; it lists the engine gaps; and it carries the state
matrix, safety rules, visual spec and packaging notes. Read it before writing code
— it is the part that prevents inventing an API. This README is the summary; that
document is authoritative on data sources.

## Architecture

The UI owns no sync logic and no index. Four data sources:

| Source | Path | Supplies |
| --- | --- | --- |
| Control socket | `$XDG_RUNTIME_DIR/proton-sync.sock` | live status, history, and the `pause` / `resume` / `syncnow` commands (newline-delimited JSON, mode 0600) |
| State files | `<local_root>/.sync/` | sync index — per-path state, sizes, timestamps, remote ids |
| Config | `~/.config/proton-sync/proton-sync.toml` | everything in Settings; UI writes it at 0600, saving restarts the daemon |
| Journal | `journalctl --user -u proton-syncd` | error detail and the "View journal" actions |

Poll `status` every 2 s focused, 10 s unfocused. **A socket error is its own UI
state, never zeroes.**

Conflict resolution needs no new IPC verb — it is file operations on the
`.proton-cloud` sidecars the engine already writes. Dry-run review shells the
daemon with `--dry-run` and renders the JSON plan.

## Screens / views

All are tabs of one 1040 × 764 window (sidebar 214 px, title bar 42 px, status
footer 30 px) unless noted.

### Overview (default tab) — option 5a

Answers, in order: is it working, does it need me, what did it just do.

- **Title bar** (42 px, chrome bg): 18 px app icon, "Proton Drive Sync" 12.5 px
  semibold, mono 11 px root pair `~/ProtonDrive ⇄ /Drive/RemoteFolder`; right side
  holds the tray-indicator button (26 px square), the Light/Dark toggle (11 px
  pill), the state pill (7 px dot + mono 11 px daemon string), and window controls.
- **Safety banner** (always present on Overview, Plan, Settings): warm-orange
  tinted strip, ▲ glyph, 12 px text — deleting on Proton Drive deletes locally and
  permanently, folders recursively.
- **Sidebar**: "Folder pair" label (10 px uppercase, 0.09em), pair card (24 px
  gradient square + name + mono path), then six nav rows (8×11 px padding, 7 px
  radius, 12.5 px): Overview, Activity (count), Conflicts (amber badge),
  Plan preview, History, Settings. Active row = `sel` bg, semibold, amber icon.
  Footer block: daemon / CLI / socket status in mono 10.5 px.
- **Status card** (flex row, 18–20 px padding, 12 px radius): 96 px hexagon with
  two arcs (amber up = uploads, cyan down = downloads) and the pending count in
  its centre; headline 17 px semibold; mono 11 px sub-line; transfer rows with a
  4 px progress bar; then a divided 140 px column with primary and secondary
  buttons (9 px vertical padding, 7 px radius) and a mono 10 px hint naming the
  underlying command.
- **Needs-you card** (210 px, amber border, hidden at zero conflicts): uppercase
  label, 26 px count, mono explanation, full-width amber "Resolve now".
- **Stat strip**: four equal tiles (`repeat(4, minmax(0,1fr))`, 12 px gap) — value
  20 px bold, label 11 px, engine field name in mono 9.5 px beneath.
- **Activity ledger**: header with filter chips (999 px pills, mono 10.5 px, counts
  derived from visible rows) and a mono provenance note; scrolling body with rows
  of action name (82 px, mono 11 px semibold, colour = direction), path (mono,
  flexes), inline "resolve" link on conflicts, and mono 11 px meta.

### Activity

Same ledger at full height, chips promoted to the header, 96 px action column.
Actions: `upload`, `download`, `move_local`, `move_remote`, `conflict`,
`skip_unsupported`, `auto_link`, `error`. Colour never carries meaning alone — the
action name is always present.

### Conflicts — options 5a (in-window) and 2a (standalone)

Rail of conflicted files (236 px) + side-by-side compare. Local pane is
orange-tinted, Proton pane cyan-tinted; changed diff lines get a tinted row and a
coloured line number. Four choices, all staged (nothing written until Apply):

| Choice | File operation |
| --- | --- |
| Keep mine | delete the sidecar; local file uploads next pass |
| Use Proton's | move the sidecar over the original |
| Keep both | rename local to `name.local.ext`, move sidecar into place |
| Decide later | nothing on disk; still counts as outstanding |

Binary files show size and timestamp only — no fabricated preview. Auto-advance to
the next unresolved file after a choice (tweakable). Decided files show a chip and
a "Change choice" affordance. **Every conflict counter in the window must read
from the same unresolved set** — sidebar badge, tab header, Needs-you card, stat
tile, ledger chip.

### Plan preview — options 5a and 1h

Dry-run JSON rendered as a summary grid of every counter plus one row per action
(action, path, entity, remote id). Destructive rows tinted red and sorted first.
1h adds a 296 px explain panel: why the planner chose the selected row, and an
"Exclude this path instead" escape hatch.

Apply is inert until the user types `DELETE` (case-insensitive) whenever
`destructive_actions > 0`; the armed button turns red, the hint names the file that
will be lost. With no destructive actions, Apply is enabled directly.

### History

Reverse-chronological `status_history` list: coloured dot, mono time (66 px),
label, mono summary. State the 20-entry in-memory limit rather than implying a
full audit log.

### Settings — options 5a (condensed) and 1k (full)

Folders (`local_root`, `remote_root`), schedule (`scan_interval_secs`,
`events_driven`), selective sync, CLI (path, `proton_timeout_secs`,
`proton_list_attempts`, test connection), service and startup.

Selective sync is one source of truth presented twice: a folder tree whose
checkboxes write exclude globs, and an advanced disclosure with the raw patterns
and live match counts. Exclude beats include; `.sync/` is always ignored; say both
in the UI. Changing a root re-bootstraps the index — force a dry run before
restart.

### Tray indicator — options 5a (live popover) and 1e (full spec)

296 px menu: status line + current file, `pending_changes` and conflict count,
then Sync now / Pause–Resume / Resolve N conflicts / Open folder / Preview plan /
Activity / Settings / Quit. Quit closes the window only — the daemon keeps running
and the item says so.

Glyph is symbolic and monochrome-safe, five states: up to date (cyan check),
syncing (two arcs), paused (bars), needs attention (amber !), daemon unreachable
(red ✕). When unreachable the menu collapses to Start service / View journal /
Settings and shows **no** counters.

### Onboarding — options 1f (stepper) and 1g (single pane)

Four steps: verify `proton-drive` is present and authenticated → choose the folder
pair → review the dry-run plan → start the service. Copy must say the first pass
is a non-destructive merge while still requiring the explicit acknowledgement that
deletions later propagate both ways. Nothing syncs before that checkbox.

### File-manager emblems — option 1l

Five 16 px symbolic emblems (synced, syncing, conflict, excluded, paused) for
Nautilus (Python extension), Nemo, Dolphin (KIO overlay). They read the sync index
plus a per-path status query and ship as per-distro packages, not inside the
Flatpak — so emblems are an enhancement, never the only signal.

## Interactions & behaviour

- Commands are optimistic in the UI but confirmed by the daemon's reply, shown as
  a dismissible strip below the safety banner; revert if no reply arrives.
- Pause/Resume is one button whose label follows state — never both.
- Conflict choices stage; the footer counts pending writes; Apply commits.
- Destructive confirmation is typed, not a checkbox, and names the specific file.
- "resolve" links anywhere open Conflicts with that file selected — no dead ends.
- Any list that can overflow scrolls (`overflow-y: auto` + `min-height: 0`). A
  clipped last row in a sync client reads as data loss.
- Follow the system light/dark preference; the theme is a palette swap over one
  layout, not two designs.
- No animation beyond the hexagon arc rotation while transfers run and the
  progress bar fill. Arcs freeze on pause.

## State management

Client state, all reachable in the prototype:

- `daemonState`: `running` | `idle` | `paused` | `authExpired` | `unreachable` | `firstRun`
  — derived from the socket reply (or its absence). Drives pill, arcs, headline,
  sub-line, buttons, banner, stat values, ledger contents, footer.
- `tab`: which view is showing.
- `theme`: `dark` | `light`, from the system preference.
- `ledgerFilter`: `all` | `uploads` | `downloads` | `moves` | `conflicts` | `skipped` | `errors`.
- `conflictSelected` + `conflictResolutions: {path → mine|proton|both|later}` —
  staged, applied together.
- `deleteGate`: the typed confirmation string; `planApplied`.
- `trayOpen`.

Derived, never stored twice: outstanding conflict count (one computation feeding
all five places it appears), chip counts (from visible rows), stat tile values
(em-dash when the socket is unreachable).

## Engine gaps

Design bets on work the daemon does not do yet. Decide each before building the
screen that needs it; the companion document has suggested resolutions.

1. **Per-file transfer percentage** — engine reports `[i/N]` file counts only. Ship
   file-count progress; do not fake a percentage.
2. **Remote folder browser** — no listing verb on the socket; shell the CLI or add
   a read-only `list` command.
3. **Applying a reviewed plan** — dry run and real run are separate invocations, so
   the applied plan may differ from the reviewed one. Re-run and re-confirm on
   divergence, or add a plan token.
4. **Live event push** — request/response only; polling is adequate here.
5. **Multiple folder pairs** — one pair today; the selector is shaped for many.
6. **Auth-state detection** — surfaces as CLI call failures; pattern-match or have
   the daemon classify it.

## Design tokens

Palette derives from the repo's own `assets/icon.svg`: **warm orange = local and
upload, cyan = remote and download**. Apply that mapping to arcs, arrows, action
names, diff panes and emblems; never decoratively. **Red is reserved exclusively
for destructive actions.**

| Role | Dark | Light |
| --- | --- | --- |
| Window bg | `#0b1220` | `#f7f8fa` |
| Card / panel | `#111b2e` | `#ffffff` |
| Chrome / sidebar | `#101a2c` / `#0e1728` | `#ffffff` / `#fbfcfd` |
| Border / soft / row | `#22304c` / `#1c2942` / `#131f36` | `#e3e7ee` / `#e3e7ee` / `#f4f6fa` |
| Text 1 / 2 | `#e6edf7` / `#c7d4ea` | `#1e293b` / `#334155` |
| Muted / faint | `#8a9bb8`, `#64748b` / `#5b6d8c` | `#64748b` / `#94a3b8` |
| Selected / border | `#1a2740` / `#2a3550` | `#eef2f8` / `#d7dde7` |
| Button bg / border / text | `#16213a` / `#2a3550` / `#c7d4ea` | `#ffffff` / `#d7dde7` / `#334155` |
| Local / upload fill | `linear-gradient(135deg,#e55b2b,#f59e0b)` | `linear-gradient(135deg,#e55b2b,#ea8c0b)` |
| Local / upload text | `#f59e0b` | `#b45309` |
| Remote / download fill | `#06b6d4` (gradient to `#3b82f6`) | `#0891b2` |
| Remote / download text | `#06b6d4` | `#0e7490` |
| Warning bg / border / text | `rgba(229,91,43,.1)` / `rgba(229,91,43,.32)` / `#f3d9c4` | `#fff7ed` / `#fed7aa` / `#7c3a11` |
| Destructive fill / text | `#ef4444` / `#fca5a5` | `#dc2626` / `#b91c1c` |
| On-accent | `#0b1220` | `#ffffff` |
| Diff highlight local / remote | `rgba(229,91,43,.13)` / `rgba(6,182,212,.13)` | `#fff3e6` / `#e8fbff` |

**Type.** IBM Plex Sans for UI; IBM Plex Mono for every path, glob, field name,
timestamp, id and count — the split tells users which strings are literal engine
values. 17 px semibold status headline · 15 px section titles · 12.5–13 px body and
controls · 11–12 px mono metadata · 10 px uppercase labels at 0.08em tracking.
Never below 10 px.

**Radii.** 12 px cards · 10 px tiles · 7 px buttons and inputs · 6 px small chips ·
999 px pills.

**Spacing.** 18–20 px card padding · 12–16 px row padding · 8–11 px list items ·
16 px between cards · 12 px between tiles · 6–9 px inside control groups.

**Fixed dimensions.** Window 1040 × 764 · sidebar 214 px · conflict rail 236 px ·
action column 140 px · tray menu 296 px · title bar 42 px · status footer 30 px.

**Shadow.** Light theme only: `0 1px 2px rgba(15,23,42,.05)` on cards; tray popover
`0 16px 44px rgba(0,0,0,.42)`. Dark theme uses borders, not shadows.

## Assets

- `assets/icon.svg` — the app icon, copied from the source repo
  (`osirison/proton-drive-sync-engine`). It is the origin of the palette. Reuse it;
  don't redraw it.
- Tray glyphs and file-manager emblems are inline SVG in the prototypes (options 1e
  and 1l) — extract and ship as symbolic icons per the freedesktop icon spec.
- Fonts: IBM Plex Sans and IBM Plex Mono (SIL Open Font License). Bundle them; do
  not rely on a webfont at runtime.

## Files in this bundle

| File | What it is |
| --- | --- |
| `Proton Sync UI.dc.html` | all design options on one canvas — start at 5a |
| `Proton Sync UI Handoff.dc.html` | the implementation contract (data sources per element, gaps, state matrix, visual spec, packaging) |
| `assets/icon.svg` | app icon from the source repo |
| `doc-page.js` | print shell used by the contract document |
| `README.md` | this file |

## Packaging

Fedora is the development target; Ubuntu and Arch are first-class. Flatpak is
primary (one build covers all three), with native packages alongside: RPM via
COPR, deb for Ubuntu, PKGBUILD on the AUR — all wrapping the same binaries plus
the systemd user unit.

Two Flatpak consequences the UI must handle: the sandbox needs explicit access to
the chosen sync folder and to the host `proton-drive` binary; and emblem
extensions can't ship inside the Flatpak. One daemon per user account is enforced
by a global lock — surface the lock holder instead of failing silently.
