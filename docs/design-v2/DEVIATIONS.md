# Deviations

Where `docs/design-v2` and the prototype disagree, and what was done about it. Resolutions follow
the precedence rule in `IMPLEMENTATION-PLAN.md` §1.3:

1. the `.md` files are normative for **tokens, rules, semantics and copy**;
2. the matching 1040 / 600 / 520 / 360 frame is normative for that screen's **layout geometry and
   per-element colour**;
3. the illustrative swatches in the prototype's "The system" header block are **not** normative.

**Status: partial.** This file currently records only what **F1** (#165) had to resolve to write
`gui/src/styles/tokens.css` — the token-level conflicts, plus what the measurement turned up on the
way. Conflicts 1–7 from `IMPLEMENTATION-PLAN.md` §1.3 are geometry and belong to the screens that
own them. The full sweep is **P0.2** (#163).

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
under-specified. `#23262D` does five jobs in dark, and light splits it three ways — measured at the
same nodes:

| Dark | Light | Sites | Role |
| --- | --- | --- | --- |
| `#23262D` | `#E6E3DE` | 4 | transfer cards, conflict version cards |
| `#23262D` | `#E0DCD5` | 8 | compact-panel edge, status chip, **quiet** buttons (`Pause`, `‹`) |
| `#23262D` | `#D6D2CB` | 5 | **secondary** buttons (`Sync now`, `Open`, `›`) |
| `#1A1D22` | `#EDEAE5` | 5 | panel borders |
| `#16181D` | `#EDEAE5` | 7 | dividers (`border-top`) |
| `#2E323A` | *(border dropped)* | 4 | primary buttons — light primary is a near-black fill, no border |

The quiet/secondary split is exactly the one `01-foundations.md` §1 already draws: **secondary** is
`bg #101216`, text `#C9D0DA`; **quiet** is `bg transparent`, text `#99A2AE`. Both frames agree.

So `tokens.css` carries `--border`, `--border-chrome`, `--btn-quiet-border` and
`--btn-secondary-border` as four tokens that all resolve to `#23262D` in dark and diverge in light.
**Do not collapse them.** Same pattern, same reason, for `--track-inert` vs `--hex-syncing-track`
(both `#191C21`; light `#EDEAE5` vs `#E4E1DB`), `--panel-raised` vs `--btn-primary-bg` (both
`#101216` at some sites; light `#FFFFFF` vs `#14161A`) and `--decision-text` vs `--destructive-text`
(both `#FF9C9C`; light `#BE123C` vs `#B91C1C`).

`#2E323A` never appears as a *border* in a drawn light frame, so `--border-strong`'s light value
`#E0DCD5` is taken from the doc's positional order and is **unverified**. Flagged for P0.2.

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

## 10. The light mapping table has no row for `--text-label`

`12-light-theme.md` maps five text tiers and omits the uppercase-label tier. Measured
`#626B78 → #6B7280`, 7 sites. Note this collapses `--text-5` and `--text-label` onto one light
value, as `--text-3` and `--text-4` also collapse onto `#4B5563`: **light has four text tiers where
dark has seven.** That is a property of the design, not an error — but S10 must not "restore" the
missing distinctions.

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

## 12. `#3E454E` is a real token and appears in no doc

7 in-scope uses, two distinct roles:

- as `color`: the sub-label under a primary button's label (`3a Conflict` → light `#B9BEC6`).
  Tokenised as `--btn-primary-quiet-text`.
- as `stroke` / `border`: an inert glyph outline and an unselected radio ring (`11a Settings`,
  `10a Glyph states`, `9a Review`). Tokenised as `--line-inert`.

None of the three frames using the second role has a light counterpart drawn, so `--line-inert`'s
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
- **Metadata inside a destructive card.** `12-light-theme.md` "Text-on-tint" says `#828B98` moves to
  `#4B5563` inside destructive/decision cards; the two such nodes in `12a Deletions light` use
  `#6B7280`. Per-element colour, so rule 2 — S3 should use `--text-5`, not `--text-4`, there.

## 17. The compact panel's seam is `--border`, not `--seam`

`2a Compact syncing` draws its seam as `linear-gradient(#0A0B0D,#23262D 30%,#23262D 70%,#0A0B0D)` —
`#23262D`, where every full-window seam uses `#2A2E36`. Both map to `#D9D5CE` in light. F3 owns the
seam helper; it needs a colour parameter, not a hard-coded `--seam`. Stops also vary by block height
(`10/90`, `12/88`, `26/74`, `30/70` measured), which `01-foundations.md` §5 already anticipates.

---

## Phase-1 capability deviations

`IMPLEMENTATION-PLAN.md` §4 lists the ten daemon capabilities the design assumes and which four are
Phase 2. Each Phase-1 fallback gets a row here as its screen lands — G1–G5 close them.
