# Deviations

Where `docs/design-v2` and the prototype disagree, and what was done about it. Resolutions follow
the precedence rule in `IMPLEMENTATION-PLAN.md` §1.3:

1. the `.md` files are normative for **tokens, rules, semantics and copy**;
2. the matching 1040 / 600 / 520 / 360 frame is normative for that screen's **layout geometry and
   per-element colour**;
3. the illustrative swatches in the prototype's "The system" header block are **not** normative.

**Status: partial.** This file currently records only what **F1** (#165) had to resolve to write
`gui/src/styles/tokens.css` — the token-level conflicts, plus what the measurement turned up on the
way. Of conflicts 1–7 in `IMPLEMENTATION-PLAN.md` §1.3, only **1** reaches a token (below); 2, 3, 5
and 6 are per-component colour or geometry owned by F2/F3, and 4 and 7 belong to their screens. The
full sweep is **P0.2** (#163).

**One caveat on the method, learned the hard way.** The lockstep walk compares each frame's
*descendants*. The frame element itself — the 1040×764 window box — is not a descendant of itself,
so its own properties were invisible in the first pass and a wrong `--border-subtle` reached the
light theme before deviation 8a caught it. Any later use of this method must include the root node.

## Method

Every row below was measured, not read. `docs/design-v2/Drive Sync.dc.html` is parsed into a node
tree; each of the seven drawn dark/light frame pairs is walked **in lockstep** and every differing
CSS property recorded as a `dark → light` substitution at a known node:

| Dark frame | Light frame | Nodes |
| --- | --- | --- |
| `2a Settled` | `12a Settled light` | 26 |
| `2a Syncing` | `12a Syncing light` | 60 |
| `4a Deletions` | `12a Deletions light` | 63 |
| `3a Conflict` | `12a Conflict light` | 75 |
| `2a Compact settled` | `12a Compact settled light` | 12 |
| `2a Compact syncing` | `12a Compact syncing light` | 36 |
| `2a Compact needs you` | `12a Compact needs light` | 13 |

The pairs align exactly — same node count, same tree path, in all seven — so a light value can be
attributed to the dark token it replaces rather than guessed from a table. That is what makes
conflicts 8 and 9 answerable instead of arguable.

---

## 8. Light border tiers — three tokens, four values

**Resolved.** `12-light-theme.md` maps *border subtle / std / strong* to `#EDEAE5` / `#E6E3DE` /
`#E0DCD5`, `#D6D2CB`. The mismatch is not a typo in the table: the **dark** palette is what is
under-specified. `#23262D` does five jobs in dark, and light splits them apart — measured at the
same nodes:

| Dark | Light | Sites | Role |
| --- | --- | --- | --- |
| `#23262D` | `#E6E3DE` | 4 | transfer cards, conflict version cards |
| `#23262D` | `#E0DCD5` | 10 | compact-panel edge, status chip, **quiet** buttons (`Pause`, `‹`) |
| `#23262D` | `#D6D2CB` | 5 | **secondary** buttons (`Sync now`, `Open`, `›`) |
| `#23262D` | `#D9D5CE` | 1 | the compact panel's seam — see deviation 17 |
| `#1A1D22` | `#EDEAE5` | 5 | panel borders |
| `#1A1D22` | `#E6E3DE` | 4 | **the window's own 1px edge** — see 8a |
| `#16181D` | `#EDEAE5` | 7 | dividers (`border-top`) |
| `#2E323A` | *(border dropped)* | 4 | primary buttons — light primary is a near-black fill, no border |

So `#23262D` splits **four** ways, not three. The counts reproduce `IMPLEMENTATION-PLAN.md` §1.3
conflict 8's independent tally of 16 / 8 / 10 / 5 uses for `#EDEAE5` / `#E6E3DE` / `#E0DCD5` /
`#D6D2CB` across the light frames.

The quiet/secondary split is the one `01-foundations.md` §1 already draws — **secondary** is
`bg #101216` *or* `#16181D`, text `#C9D0DA`/`#E8EBF0`; **quiet** is `bg transparent`, text
`#99A2AE`. Note the frames use both secondary fills (`Sync now` in `2a Settled` is drawn
`background:transparent`, which is `IMPLEMENTATION-PLAN.md` §1.3 conflict 4, still open; the two
compact `Open` buttons use `#16181D`), so `--btn-secondary-bg-alt` exists alongside
`--btn-secondary-bg`.

So `tokens.css` carries `--border`, `--border-chrome`, `--btn-quiet-border` and
`--btn-secondary-border` as four tokens that all resolve to `#23262D` in dark and diverge in light.
**Do not collapse them.** Same pattern, same reason, for `--border-subtle` vs `--border-window`
(both `#1A1D22`; light `#EDEAE5` vs `#E6E3DE` — 8a), `--track-inert` vs `--hex-syncing-track` (both
`#191C21`; light `#EDEAE5` vs `#E4E1DB`), `--panel-alt` vs `--compact-row` (both `#0E0F12`; light
`#FDFCFA` vs `#FFFFFF` — deviation 16), `--text-2` vs `--btn-secondary-text` (both `#C9D0DA`; light
`#374151` vs `#14161A`) and `--decision-text` vs `--destructive-text` (both `#FF9C9C`; light
`#BE123C` vs `#B91C1C`).

`#2E323A` never appears as a *border* in a drawn light frame, so `--border-strong`'s light value
`#E0DCD5` is taken from the doc's positional order and is **unverified**. Flagged for P0.2.

### 8a. The window's own border

`#1A1D22` splits in light as well: `#EDEAE5` at the five panel/divider sites, and **`#E6E3DE` at the
1040×764 window root** of `2a Settled`, `2a Syncing`, `4a Deletions` and `3a Conflict`. Their light
twins all draw `border:1px solid #E6E3DE`. `01-foundations.md` §1 lists `#1A1D22` as "Panel
borders", which is why the doc alone cannot separate them — and why the first pass got it wrong.
`--border-window` carries this; do not reach for `--border-subtle` (wrong in light) or `--border`
(wrong in dark) for the window edge.

### 8b. The conflict screen's choice buttons are a second decision-button tint

`--decision-btn-*` (`.4`/`.1` dark → `.38`/`.06` light) matches `4a Deletions`'
`Move to Proton's Trash` and `2a Compact needs you`' `Review them`. The two choice buttons in
`3a Conflict` (`→ Keep mine`, `← Use Proton's`) are drawn `rgba(255,107,107,.4)` /
`rgba(255,107,107,.07)` → `rgba(190,18,60,.35)` / `rgba(190,18,60,.04)`. The `.07` fill is
`04-conflicts.md`'s own value, so this is a second documented variant rather than a drawing slip:
`--decision-choice-border` / `--decision-choice-bg`.

## 9. Light disabled text — one tier, not two

**Resolved, and the plan's framing was wrong.** `--text-disabled` is a clean one-to-one
`#4E5661 → #B9BEC6`. `#9CA3AF` is **not** a second disabled tier:

- `#B9BEC6` ← `#4E5661` at the disabled `‹` in `3a Conflict`. Correct.
- `#B9BEC6` ← `#3E454E` at the sub-label inside the primary `Keep both` button — a different role
  (text **on** the primary fill), not a disabled tier. See deviation 12.
- `#9CA3AF` ← `#6D7783` (`--text-5`) at exactly one node: the `→` in a *queued* transfer row in
  `12a Syncing light`. **Sixteen** other `#6D7783` nodes map to `#6B7280`. One node against sixteen
  is a drawing inconsistency, not a tier.

`#9CA3AF` is therefore not a text token. It is reused as `--btn-primary-disabled-text` in light,
where nothing was drawn (see deviation 15).

## 10. The light mapping table omits two text tiers

`12-light-theme.md` maps five dark text values and names neither `#626B78` (`--text-label`) nor
`#E8EBF0` (`--text-bright`) anywhere in the file. Measured: `#626B78 → #6B7280` (7 sites) and
`#E8EBF0 → #14161A` (6 as `color`, 2 as `stroke`).

Note how much light collapses: `--text` and `--text-bright` both land on `#14161A`, `--text-3` and
`--text-4` both on `#4B5563`, `--text-5` and `--text-label` both on `#6B7280`. **Light has four text
tiers where dark has seven.** That is a property of the design, not an error — but S10 must not
"restore" the missing distinctions.

## 11. Font weights — the frames contradict the prose

`01-foundations.md` §2: *"Weights in use: 500, 600, 700 (sans); 400, 600 (mono). Nothing is
400-weight sans except long body paragraphs."* Measured over every frame:

| | 400 | 500 | 600 | 700 |
| --- | --- | --- | --- | --- |
| Instrument Sans | **504** | 13 | 244 | **0** |
| IBM Plex Mono | 352 | **0** | 19 | 0 |

Sans 400 is the most common weight in the design; 700 does not occur. All 25 `<strong>` elements
carry an explicit inline `font-weight:600`, so no UA default is hiding a 700 anywhere, and all 30
`<em>` carry `font-style:normal`, so no italic face is needed.

**Resolution:** bundle sans 400/500/600/700 and mono 400/600 (`gui/src/fonts/README.md`). 700 is
kept because §2 names it and its absence would be invisible — `font-weight` computes to 700 whether
or not a 700 face exists, so the F8 style gate would pass on a synthetic bold.

## 12. `#3E454E` is a real token, named in four docs but in no token table

`01-foundations.md` §1 has no row for it, which is what matters for `tokens.css` — but it is not
undocumented: `04-conflicts.md` gives it twice (the `Keep both` body text; the `not in your version`
label in the diff view), `08-settings.md` as a `1.5px` radio ring, `09-onboarding.md` as a 13px
hexagon outline, and `10-tray.md` as a syncing track. 7 in-scope uses, two distinct roles:

- as `color`, 2 nodes: the sub-label under a primary button's label (`3a Conflict`, → light
  `#B9BEC6`) and the absent-line text in `3a Conflict diff`. Tokenised as
  `--btn-primary-quiet-text`; the diff-view role shares the value but not the meaning, so S2 should
  expect to name its own.
- as `stroke` / `border`, 5 nodes across 4 frames: an inert glyph outline and an unselected radio
  ring (`11a Settings` ×2, `10a Glyph states`, `9a Review`, `8a Deletions tab`). Tokenised as
  `--line-inert`.

None of the four frames using the second role has a light counterpart drawn, so `--line-inert`'s
light value (`#C7CBD2`, borrowed from the settled-hexagon track) is **unverified**. Flagged for P0.2.

## 13. Settled-glow alpha is `.05`, not `.055`

`01-foundations.md` §7 and `12-light-theme.md` both say `rgba(232,235,240,.055)`. That value occurs
exactly once in the prototype — in `1c Main`, a round-one frame that is out of scope. The in-scope
`2a Settled` uses `.05`, and its light twin `12a Settled light` uses `rgba(20,22,26,.045)`, which
the doc gets right. Frame wins (rule 2): `--glow-settled` is `.05` / `.045`.

## 14. Tray-panel shadow is `.6`, not `.62`

`01-foundations.md` §3 specifies `0 22px 54px rgba(0,0,0,.62)`. The four actual tray panels
(`10a Settled/Syncing/Offline/Paused`) use `.6`; `.62` occurs once, on the panel in `10a In situ`,
which is a desktop mock sitting on a wallpaper. Frame majority wins: `--shadow-tray-panel` is `.6`.

## 15. Light values with no drawn frame

These are in `tokens.css` because a token needs a value in both themes, but nothing measured them.
Each is a guess constrained by the surrounding ramp. **S10 must verify or replace them, and P0.2
should ask the designer:**

| Token | Light value | Basis |
| --- | --- | --- |
| `--border-strong` | `#E0DCD5` | doc's positional order (deviation 8) |
| `--line-inert` | `#C7CBD2` | settled-hexagon track (deviation 12) |
| `--btn-primary-disabled-bg` | `#E0DCD5` | doc gives dark `#2A2E36` only |
| `--btn-primary-disabled-text` | `#9CA3AF` | the one light grey in the frames that is not a tier |
| `--hex-paused-track` / `--hex-paused-bars` | `#C7CBD2` / `#4B5563` | no light `10a Paused` exists |
| `--btn-destructive-text` | `#fff` | `4a Armed` has no light frame; white on `#DC2626` holds |
| `--shadow-banner` | `.4` alpha | `12-light-theme.md` says light uses `.4–.45` |

## 16. Two per-element light values the token can't carry

Recorded so the screens that own them do not read them as token bugs:

- **`--panel-raised` in light.** The doc maps `#101216 → #FDFCFA`; the frames use `#FFFFFF` at all
  6 sites. The token follows the frames. `#FDFCFA` does occur once — as the light twin of `#0E0F12`
  (`--panel-alt`) in `12a Syncing light` — which is where the token puts it.
- **The compact panel's transfer rows.** `#0E0F12` → `#FDFCFA` at the queued row in
  `12a Syncing light`, but → `#FFFFFF` at the compact panel's two transfer rows
  (`12a Compact syncing light`), where the 1040 window uses `#101216`. `--panel-alt` follows the
  doc and the window; `--compact-row` carries the compact panel's value. F6 uses the latter.
- **Metadata inside a destructive card.** `12-light-theme.md` "Text-on-tint" says `#828B98` moves to
  `#4B5563` inside destructive/decision cards; the two such nodes in `12a Deletions light` use
  `#6B7280`. Per-element colour, so rule 2 — S3 should use `--text-5`, not `--text-4`, there.

## 17. The compact panel's seam is `--border`, not `--seam`

`2a Compact syncing` draws its seam as `linear-gradient(#0A0B0D,#23262D 30%,#23262D 70%,#0A0B0D)` —
`#23262D`, where every full-window seam uses `#2A2E36`. Both map to `#D9D5CE` in light. F3 owns the
seam helper; it needs a colour parameter, not a hard-coded `--seam`. Stops also vary by block height
(`10/90`, `12/88`, `26/74`, `30/70` measured), which `01-foundations.md` §5 already anticipates.

## 18. The paused hexagon track is `#2E323A` (§1.3 conflict 1, closed)

`01-foundations.md` §6 says the paused track is `#2A2E36`; `10a Paused` draws
`stroke="#2E323A" stroke-dasharray="14 12"`. Frame wins (rule 2), so `--hex-paused-track` is
`#2E323A` — which makes it equal to `--hex-settled-track` in dark. They are separate tokens because
nothing measured them in light and they need not move together. Recorded here because `tokens.css`
promises that every departure from a doc table is written down.

## 19. `base.css` applies `box-sizing: border-box` globally; the prototype does not

The prototype opts in inline, 53 times, but **not** on the 360px compact shell
(`width:360px; border:1px solid #23262D; border-radius:14px` at 12 in-scope frames), which is
therefore drawn 362px wide overall. The one 360px node that does opt in is the `12a Tray light`
specimen card, which is not a product surface.

The reset is kept: `01-foundations.md` §4 specifies the panel as "360px wide", hand-written CSS in
2026 assumes border-box, and the shipped `app.css` already sets it. The consequence is a systematic
2px difference on any element that pairs an explicit width or height with a border or padding and
was drawn without `box-sizing`. **F6 must write the compact shell as 362px** (or set
`box-sizing: content-box` on it) to reproduce the frame, and F8's fixtures should be generated after
that decision, not before. Everywhere else the reset is inert — `box-sizing` does nothing to an
element whose size is `auto` or flex-derived.

---

## Phase-1 capability deviations

`IMPLEMENTATION-PLAN.md` §4 lists the ten daemon capabilities the design assumes and which four are
Phase 2. Each Phase-1 fallback gets a row here as its screen lands — G1–G5 close them.
