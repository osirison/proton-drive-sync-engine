# Handoff: Proton Drive Sync — desktop UI redesign

## Overview

A complete redesign of the Proton Drive Sync desktop GUI (the Tauri app in `gui/` of
`osirison/proton-drive-sync-engine`). Eleven screens plus the system tray, notifications and a
light theme, all built on one idea — **the seam**: the window is split down the middle, your
computer on the left, Proton Drive on the right, and everything the app tells you is positioned
according to which side it belongs to.

Target platform is **Linux only** (GNOME/KDE, systemd user service, the `proton-drive` CLI).
There is no macOS or Windows copy in this spec; "this computer" is used throughout rather than
any OS-specific phrasing.

## About the design files

The files in this bundle are **design references created in HTML** — prototypes showing intended
look and behaviour. They are **not production code to copy directly**.

The existing app is vanilla JS with hand-rolled DOM helpers (`gui/src/js/components.js`),
CSS custom properties in `gui/src/styles/tokens.css`, and a screen-module pattern in
`gui/src/js/screens/*.js`. **Recreate these designs in that environment**, following its
established patterns — extend `tokens.css` with the new values in `01-foundations.md`, keep the
screen-module structure, keep the existing `api.js` daemon interface. Do not introduce a
framework to match the prototypes; the prototypes only happen to be React-flavoured because of
the tool they were drawn in.

Where a design implies a daemon capability the engine does not have yet (noted per screen), treat
that as a product question, not a UI detail to invent.

## Fidelity

**High fidelity.** Every colour, type size, weight, letter-spacing, radius, gap and string in
this spec is the intended final value, measured from the prototypes. Recreate pixel-perfectly.

Two known open items, flagged rather than hidden:
- The `#6D7783` quiet-caption tier measures **4.33:1** on `#0A0B0D` — just under WCAG AA for
  small text. It is used for mono captions at 10.5–12px. Either accept it as a deliberate quiet
  tier or lift to `#767F8C` (4.6:1) before build. Decide once, globally.
- Seven of the eleven screens have no light-theme frame drawn yet. `12-light-theme.md` gives the
  complete token mapping needed to finish them mechanically.

## How to read this bundle

| File | What's in it |
| --- | --- |
| `01-foundations.md` | Colour system, type scale, spacing, radii, the seam rules, the hexagon, animation, symbols |
| `02-shell.md` | Window chrome, header, footer nav, the 360px compact panel, layout skeleton |
| `03-main-screen.md` | The main screen, three states |
| `04-conflicts.md` | Both-changed resolution |
| `05-deletions.md` | The delete approval queue |
| `06-plan.md` | Plan a sync (dry run) |
| `07-activity.md` | Activity — file lookup, never-synced, sync passes |
| `08-settings.md` | Settings, four tabs |
| `09-onboarding.md` | First run, two steps |
| `10-tray.md` | System tray glyph + panel |
| `11-notifications.md` | What interrupts, and what deliberately doesn't |
| `12-light-theme.md` | Light palette and the mapping rule |
| `13-copy-deck.md` | Every user-visible string, verbatim |
| `14-behaviour-and-state.md` | State model, transitions, daemon fields, error states |
| `Drive Sync.dc.html` | The design prototype — open in a browser |
| `Current UI.dc.html` | The existing build, recreated, for before/after |
| `icon.svg` | The brand hexagon, from `gui/src/assets/icon.svg` |

## The five rules that hold it together

If a detail is ever ambiguous, these decide it.

1. **The seam is earned.** The centre hairline, the two side labels and the transfer rows are
   drawn only when there is traffic or a decision. At rest the window is silent. The seam always
   stops above any full-width band.
2. **Colour means motion or a decision.** Warm = leaving this computer. Cool = arriving from
   Proton. Outlined crimson = a person must decide. Solid red = irreversible, right now.
   **Settled has no colour at all.**
3. **Two tiers of language.** Plain sentences for people; IBM Plex Mono for anything the daemon
   owns — paths, config keys, ids, byte counts, error strings. Machine detail lives one click
   behind *Details*, never as a permanent caption.
4. **The anchors never move.** The hexagon sits at the same point in every state of a screen, and
   the four footer doors never move. The window changes volume, not shape.
5. **Nothing destructive is one click away.** Keep is always the default and never needs
   confirming. Delete needs a typed confirmation and a window where you can see what you'd lose.
   Destructive actions never appear as notification buttons.

## Assets

- `icon.svg` — the brand hexagon, from `gui/src/assets/icon.svg`. Identical geometry; the copy
  here has been reformatted by the design tool (comments stripped, self-closing tags expanded),
  so diff the `points` attribute rather than the file. Used as the 20px app mark in every
  window header. **Pointy-top orientation** (vertex top and bottom).
- All status hexagons are inline SVG derived from that same geometry — see `01-foundations.md`.
- No other images. Every glyph in the UI is either inline SVG or a Unicode character; see the
  symbol table in `01-foundations.md`, which also recommends replacing them with a real icon set.

## Fonts

Instrument Sans and IBM Plex Mono, both on Google Fonts. The existing `tokens.css` specifies IBM
Plex Sans but the woff2 files were never bundled, so the app currently falls back to system fonts
— **bundle the fonts this time** rather than relying on the CDN, since this is a desktop app that
must render offline.
