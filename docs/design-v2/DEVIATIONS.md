# Deviations

Where `docs/design-v2` and the prototype disagree, and what was done about it. The five decisions
that were taken once and apply everywhere are in `DECISIONS.md`; this file is the per-conflict
record under them. Resolutions follow the precedence rule in `IMPLEMENTATION-PLAN.md` §1.3:

1. the `.md` files are normative for **tokens, rules, semantics and copy**;
2. the matching 1040 / 600 / 520 / 360 frame is normative for that screen's **layout geometry and
   per-element colour**;
3. the illustrative swatches in the prototype's "The system" header block are **not** normative.

**Status: partial.** Sections are numbered in the order they were resolved and grouped by the task
that resolved them: **§8–§19 F1** (#165, `tokens.css`), **§20–§31 F2** (#166, the hexagon),
**§32–§39 F3** (#167, the seam), **§40–§46 F4** (#168, the shell), **§47–§51 F7/F8** (#171/#172, the
copy deck and the fidelity harness), **§52–§57 F5** (#169, controls/rows/bands/dialog),
**§58–§59 F6** (#170, the compact panel), **§60–§62 F9** (#173, the per-frame fixtures),
**§63–§67 S1** (#180, the main screen). Of conflicts 1–7 in `IMPLEMENTATION-PLAN.md` §1.3, **1** reaches a
token (§18) and **2, 3, 5 and 6** are per-component colour or geometry that F2 and F3 have now
measured; 4 and 7 belong to their screens. The full sweep is **P0.2** (#163).

**One caveat on the method, learned the hard way.** The lockstep walk compares each frame's
_descendants_. The frame element itself — the 1040×764 window box — is not a descendant of itself,
so its own properties were invisible in the first pass and a wrong `--border-subtle` reached the
light theme before deviation 8a caught it. Any later use of this method must include the root node.

## Method

Every row below was measured, not read. `docs/design-v2/Drive Sync.dc.html` is parsed into a node
tree; each of the seven drawn dark/light frame pairs is walked **in lockstep** and every differing
CSS property recorded as a `dark → light` substitution at a known node:

| Dark frame             | Light frame                 | Nodes |
| ---------------------- | --------------------------- | ----- |
| `2a Settled`           | `12a Settled light`         | 26    |
| `2a Syncing`           | `12a Syncing light`         | 60    |
| `4a Deletions`         | `12a Deletions light`       | 63    |
| `3a Conflict`          | `12a Conflict light`        | 75    |
| `2a Compact settled`   | `12a Compact settled light` | 12    |
| `2a Compact syncing`   | `12a Compact syncing light` | 36    |
| `2a Compact needs you` | `12a Compact needs light`   | 13    |

The pairs align exactly — same node count, same tree path, in all seven — so a light value can be
attributed to the dark token it replaces rather than guessed from a table. That is what makes
conflicts 8 and 9 answerable instead of arguable.

---

## 8. Light border tiers — three tokens, four values

**Resolved.** `12-light-theme.md` maps _border subtle / std / strong_ to `#EDEAE5` / `#E6E3DE` /
`#E0DCD5`, `#D6D2CB`. The mismatch is not a typo in the table: the **dark** palette is what is
under-specified. `#23262D` does five jobs in dark, and light splits them apart — measured at the
same nodes:

| Dark      | Light              | Sites | Role                                                              |
| --------- | ------------------ | ----- | ----------------------------------------------------------------- |
| `#23262D` | `#E6E3DE`          | 4     | transfer cards, conflict version cards                            |
| `#23262D` | `#E0DCD5`          | 10    | compact-panel edge, status chip, **quiet** buttons (`Pause`, `‹`) |
| `#23262D` | `#D6D2CB`          | 5     | **secondary** buttons (`Sync now`, `Open`, `›`)                   |
| `#23262D` | `#D9D5CE`          | 1     | the compact panel's seam — see deviation 17                       |
| `#1A1D22` | `#EDEAE5`          | 5     | panel borders                                                     |
| `#1A1D22` | `#E6E3DE`          | 4     | **the window's own 1px edge** — see 8a                            |
| `#16181D` | `#EDEAE5`          | 7     | dividers (`border-top`)                                           |
| `#2E323A` | _(border dropped)_ | 4     | primary buttons — light primary is a near-black fill, no border   |

So `#23262D` splits **four** ways, not three. The counts reproduce `IMPLEMENTATION-PLAN.md` §1.3
conflict 8's independent tally of 16 / 8 / 10 / 5 uses for `#EDEAE5` / `#E6E3DE` / `#E0DCD5` /
`#D6D2CB` across the light frames.

The quiet/secondary split is the one `01-foundations.md` §1 already draws — **secondary** is
`bg #101216` _or_ `#16181D`, text `#C9D0DA`/`#E8EBF0`; **quiet** is `bg transparent`, text
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

`#2E323A` never appears as a _border_ in a drawn light frame, so `--border-strong`'s light value
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
- `#9CA3AF` ← `#6D7783` (`--text-5`) at exactly one node: the `→` in a _queued_ transfer row in
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

`01-foundations.md` §2: _"Weights in use: 500, 600, 700 (sans); 400, 600 (mono). Nothing is
400-weight sans except long body paragraphs."_ Measured over every frame:

|                 | 400     | 500   | 600 | 700   |
| --------------- | ------- | ----- | --- | ----- |
| Instrument Sans | **504** | 13    | 244 | **0** |
| IBM Plex Mono   | 352     | **0** | 19  | 0     |

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
Each is a guess constrained by the surrounding ramp.

**S10 walked all eight light frames against their twins and none of these seven is drawn in light,
so all seven stand — as CHOSEN, not as verified.** §91b has the census that says so, the wider list
of 33 it belongs to, and the one reason `--btn-primary-disabled-text` reads as "observed" and is not.
P0.2 should still ask the designer:

| Token                                      | Light value           | Basis                                                   |
| ------------------------------------------ | --------------------- | ------------------------------------------------------- |
| `--border-strong`                          | `#E0DCD5`             | doc's positional order (deviation 8)                    |
| `--line-inert`                             | `#C7CBD2`             | settled-hexagon track (deviation 12)                    |
| `--btn-primary-disabled-bg`                | `#E0DCD5`             | doc gives dark `#2A2E36` only                           |
| `--btn-primary-disabled-text`              | `#9CA3AF`             | the one light grey in the frames that is not a tier     |
| `--hex-paused-track` / `--hex-paused-bars` | `#C7CBD2` / `#4B5563` | no light `10a Paused` exists                            |
| `--btn-destructive-text`                   | `#fff`                | `4a Armed` has no light frame; white on `#DC2626` holds |
| `--shadow-banner`                          | `.4` alpha            | `12-light-theme.md` says light uses `.4–.45`            |

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

## The hexagon (F2)

Measured across all 53 in-scope hexagons in the prototype. Sizes come from `style="width:Npx"` on
the `<svg>`, never `width`/`height` attributes, and **every** `stroke-width`, `font-size`, `r` and
`y` below is in user units on the 120 viewBox — rendered px is `value x size / 120`.

## 20. §6's size list is the round-one list

`176`, `132`, `74` and `46` occur **only** in the out-of-scope `1a`/`1b`/`1c` frames, while **`64`**
(`4a Compact`) and **`48`** (`7a File pending`) are drawn in scope and missing from the list. The
in-scope set is the 17 sizes in `ui/hexagon.js`. Frames win (rule 2).

## 21. Stroke width is a lookup table; no formula exists

Two structural disproofs, not a poor fit: it is **not single-valued** (80px is drawn at 4.4 in
`4a Empty` and 4.6 in `9a Review` — same state, same construction) and **not monotonic** (48 -> 5.4
but the smaller 44 -> 5). §6's bands are also wrong at both ends: `3a Conflicts cleared` draws 96px
at **4.2**, below the stated 4.4 floor, and 72px is claimed by both the `4.4-4.6` and `5-5.4` bands.

`strokeForSize` therefore **throws** on an unmeasured size rather than interpolating — the best
power-law fit still errs 17-18%, far outside the +/-0.5px the fidelity gate allows, so a guess would
be a plausible wrong number. 80px has no default at all; the caller must pass one.

Two sizes are drawn at two widths by _family_: 72px is 4.5 in the `2a`/`12a` compacts but 4.6 in the
`10a` tray panels, and 34px is 6 in-window but 7 in every `11a` notification. The 72px split
contradicts §1.2's "shared component; the tray reuses it" — **F2 and F6 need a joint designer call**.

## 22. "The mark reads as the same weight at every size" is not what is drawn

§6. Rendered stroke falls monotonically from 4.76px at 168 to 1.05px at 14, while _relative_ weight
rises from 2.8% to 10% of the mark. That is an optical-compensation ramp, and a good one — but it is
the opposite of constant, so the sentence cannot be implemented as written.

## 23. The perimeter is 303.0115, not ~297 — and the dash arrays are not tuned to it

§6 says _"Perimeter = 297 units - the number the dash arrays are tuned against."_ 297 is
`6 x 49.5275`, which assumes a **regular** hexagon. This one is not: the two vertical sides are 52.5
units, the four slants 49.5275 and 49.4783. The true perimeter is **303.0115** (verified twice,
independently).

The dash arrays are tuned against something else entirely: `62+238`, `40+260` and `70+230` all sum
to **300**, which is exactly the `stroke-dashoffset` travel in `hexup`/`hexdn`. _Dash period equals
offset travel_ is what makes the loop seamless. The 3.0115-unit remainder leaves a permanent stub of
"on" at the path start (the top vertex) — visible in every drawn frame. **Do not retune**: matching
the true perimeter would remove the stub and break the F8 gate against the frames. Doc error, code
unchanged.

## 24. There is no crimson hero — `2a Needs you` draws the _syncing_ mark

The most likely thing to build wrong. At 168px, `2a Needs you` is byte-identical to `2a Syncing`
apart from its gradient ids: same `#191C21` track, same two travelling segments, same neutral
`#F2F4F7` numeral. Not a bug — `03-main-screen.md` says _"the count in the hexagon is transfers, not
decisions"_, and the attention band carries the decisions. **The crimson mark exists only at
<=72px.** §6's five-state table reads as though every state has a hero form; it does not.

## 25. The seam mask is orthogonal to state, and the plan is wrong in both directions

IMPLEMENTATION-PLAN §5's F2 row says _"syncing track carries `fill:<surface>`"_. Measured, `fill`
tracks whether the mark sits over the seam, not the state: **18 in-scope hexagons carry a fill** and
they include settled marks (`9a Review` 80, `5a Plan safe` 88, `7a Activity quiet`/`File lookup` 52)
and needs-you marks (`3a Conflict` 44) — while `2a Settled` and seven other syncing-less frames carry
none. It is not derivable from "does this frame have a seam" either: `7a File pending` and
`3a Conflict diff` are masked with no seam element at all. Hence an explicit `masked` flag.

## 26. Four small constructions the docs do not carry

- **The settled check is heavier than its track — at 168px only** (3.6 vs 3.4, in both `2a Settled`
  and `12a Settled light`). Equal at all 12 other settled sizes. A hand-tuned hero special case.
- **The check is dropped entirely at <=20px.** The tray settled glyph and the 13px bullet are bare
  outlines. Reusing the panel construction at tray size ships a checkmark that is not in the design.
- **Paused `opacity` sits on the `<svg>` root, not the track** (`10a Paused`), so the bars dim too.
  §6 writes it inside the _Track_ cell, which reads as a path property; applying it there leaves the
  bars 45% too bright. `.55` at 72px, `.45` at tray size, and the tray form has **no bars**.
- **The unreachable strike has two forms**: `M40 40 L80 80` at 34/72, and the longer
  `M38 38 L82 82` at <=20px so it still reads at tray size. §6 and `10-tray.md` are each right for
  their own size class.

## 27. Strike and warning-bar paths carry no `fill` and no `stroke-linejoin`

Verified on all 8 in-scope instances: they set **only** `stroke-linecap="round"`. Emitting
`fill="none"` there is visually identical and fails the F8 style gate on a node that looks correct.
The settled check, by contrast, carries both `linecap` and `linejoin`.

The warning variant is also a **sixth construction**, not a modifier: it replaces the numeral or
check entirely, and has three geometries (`M60 38 L60 64` + `cy=79 r=4.6` at 34 decision;
`M60 36 L60 66` + `cy=80` at 34 destructive; `M60 38 L60 66` sw6 + `r=3.6` at 104). Both destructive
instances carry a `rgba(255,59,59,.08)` tint — the only fill in the whole set that is neither `none`
nor a surface colour, and **not** `--destructive-bg`, which is `.06`.

## 28. `#2A2E36` as a tray-glyph track had no token

`10a Glyph states` draws the colour syncing glyph's track at `#2A2E36`, which exists in `tokens.css`
only as `--seam` and `--btn-primary-disabled-bg` — both wrong roles, and the file header warns
against borrowing. Added as **`--hex-glyph-track`**. No light syncing glyph is drawn, so its light
value (`#E4E1DB`, the light `--hex-syncing-track` — same role, an inert track under a travelling
segment) joins the unverified list in §15.

## 29. `blip` is not the hexagon's, and `breathe` drives the screen's element

IMPLEMENTATION-PLAN §5's F2 row lists both keyframes. `blip` never touches a hexagon in any frame
(it drives a 6px status dot and a 1.5x15px caret) and belongs to F4/F5. `breathe` drives the settled
_glow_ — a **sibling** 480px div behind a 168px mark, present on only 2 of 9 in-scope settled
hexagons. A component that emitted it would paint 156px outside its own box and fail the fit gate,
so F2 ships the keyframe and S1 places the div.

## 30. Reduced motion: rule 1 governs, and one case needs a designer

The prototype contains **zero** `prefers-reduced-motion` rules, so no frame exists and rule 2 cannot
apply. §7's wording is normative: _"drop the travelling segments to a static 40%-opacity coloured
outline"_ — which means the **dasharray goes too**, since a frozen dash is a segment, not an outline.
Two consequences worth stating:

- **The paused dasharray must survive.** It is static geometry carrying the state's meaning, not an
  animation; a blanket "strip dasharray under reduced motion" rule destroys the state.
- **The glow must be pinned to `.45`, not merely un-animated.** Removing the animation returns it to
  `opacity:1` — brighter than the keyframe's own `.8` peak, so honouring the preference would make
  it _more_ prominent.

**Open.** With both segments un-dashed, the cool path (painted last) fully occludes the warm one and
the direction contract disappears. §7's sentence was written for the single-segment case. Options:
render only the warm path at `.4`; both with the cool at `.2`; or freeze the dashes mid-travel.
Needs a designer call.

## 31. `gui/src/styles/hexagon.css` is a new stylesheet §3.4 does not list

`@keyframes` cannot be inline, so the mark needs a stylesheet, and it must not be today's
`components.css` — that is the v1 file F4 deletes, and §3.4's `components.css` is a different future
file with the same name. Keyframes written into today's copy would vanish at F4 and the mark would
silently stop animating: no error, no lint signal. Recorded so F4 folds this in deliberately rather
than by name collision. It also requires a `<link>` in `index.html`; omitting that is the same
silent failure.

## The seam (F3)

Measured differently from the hexagon, and it had to be. A hexagon is a self-contained `<svg>` whose
attributes can be read out of the source; a seam is a 1px box whose height, and therefore every one
of its gradient stops, is produced by layout. So the prototype was **rendered** — loaded into a real
engine at 1400px and read off `getBoundingClientRect` — and the tokens were injected into that same
page so `var(--seam)` and a literal `#2A2E36` resolve to the same `rgb()` and can be compared
directly. All 22 drawn seams were found by the same structural test used below
(`width:1px` + a `linear-gradient` with no angle), which agrees exactly with the 22 no-angle
gradients in the raw text.

**Roundtrip.** Each of the 16 sites in `ui/seam.js` was rebuilt with `renderSeam({site})`, inserted
next to the seam it reproduces in the prototype's own containing block, and required to match on
computed `background-image` and on all four numbers of the laid-out box. 20 of 20 drawn seams —
including the four light twins, checked under `data-theme="light"` — match exactly. That harness
caught one real defect before this landed: `??` and destructuring defaults both treat the `null`
that means "this end is cut" as absent, which silently restored a 74% fade-out on all six cut ends.

## 32. Rule 1 over-predicts; the 16 drawn sites are the authority

§5 rule 1: the seam is drawn _"when data is moving, or when a decision has two sides"_. Taken as a
predicate that would put a seam on `2a Compact needs you`, `12a Compact needs light`, `4a Compact`
and `3a Conflicts cleared` — every one a two-sided decision, none of which draws one. 32 in-scope
frames have no seam. §25's finding points the same way from the other side: `7a File pending` and
`3a Conflict diff` carry the hexagon's seam mask with no seam element anywhere in the frame, so a
masked mark does not imply a seam either.

The rule is right about _intent_ and cannot be mechanised. `auditSeams()` therefore checks rules 2
and 3 and deliberately does not check rule 1 — a rule-1 check would report four of the design's own
screens as violations. S1–S11 should add a seam where `SEAM_SITES` has a row and nowhere else.

## 33. It does touch an edge — there are three gradient shapes, not one

§5: _"It fades in and out at both ends against the surface colour — it never touches an edge."_ Six
of the 20 run at **full colour** into one end:

| Shape          | Sites                                                                                | Form               |
| -------------- | ------------------------------------------------------------------------------------ | ------------------ |
| both ends fade | 10 sites, 14 drawn                                                                   | `S, L a%, L b%, S` |
| bottom cut     | `5a Plan`, `5a Plan safe` (hero), `7a Activity quiet`, `7a File lookup`, `9a Review` | `S, L a%, L 100%`  |
| top cut        | `5a Plan safe` (list)                                                                | `L, L b%, S`       |

The seam fades where it **stops**, not where it is handed to the block below. `5a Plan safe` makes
the point twice over: it draws **two** seam elements, in two different blocks, that overlap by 66px
(`bottom:-40px` on the upper, `top:-40px` on the lower) and read as one unbroken line across the gap.
The lower one has no top fade at all, which is the whole reason the joint is invisible.

Emitting four stops always and writing `100` into one of them reproduces the bottom-cut form by
accident and gets the top-cut form wrong — `S, L 0%, L 82%, S` still fades up from the surface over
the first pixel row, which is exactly the seam the pair exists to remove. `ui/seam.js` expresses a
cut end as `null`, not as `100`.

Related: the seam routinely **overflows its container**, which the prose does not mention. `2a
Syncing` runs 150px past the bottom of its 394px hero, down through the transfer rows; `2a Needs
you` 114px; `9a Review` 56px; `5a Plan safe` 40px; `7a File lookup` 24px. A screen that clips its
seam to its own padding box has not drawn this design.

### 33a. §1.3 conflict 6 is closed, and it was not a conflict

`IMPLEMENTATION-PLAN.md` §1.3 row 6 reads the doc's `-114..-150px` as a range the frame narrows to
`-150px`. It is not a range: **both values are drawn**, at two different sites, and rule 2 is what
separates them. `2a Needs you`'s attention band starts 567px into the frame; its seam ends at 561px
— a 6px clearance — which is what `-114px` buys. `2a Syncing` has no band there and runs the full
`-150px` to the bottom of the transfer rows. `mainHero` and `mainHeroAttention` are separate rows of
`SEAM_SITES` for that reason, and they differ in their fade-out too (74 vs 78).

Worth noting for S1: the attention band is 976px wide inside a 1040px frame, so it is not
full-width in the sense `auditSeams()` tests for. Rule 2's "spans the window" is about _reading_ as
a band, and an inset card with a crimson border reads as one. The audit will not catch a seam run
into it; the site rows will.

## 34. The stops are not a function of height, and the fade is a quarter, not an eighth

§5: _"The percentage stops vary by block height (10–30% in, 70–90% out); pick stops that put full
opacity across the content and fade over roughly the top and bottom eighth."_ Three claims; the
envelope is right, the other two are not.

**Not a function of height** — two clean disproofs, both between same-shape symmetric seams:

|                | height | stops | fade-in |
| -------------- | ------ | ----- | ------- |
| `2a Needs you` | 508px  | 26/78 | 132.1px |
| `4a Deletions` | 514px  | 10/90 | 51.4px  |
| `5a Checking`  | 543px  | 30/70 | 162.9px |
| `2a Syncing`   | 544px  | 26/74 | 141.4px |

Six pixels apart with a 132px fade against a 51px one, and one pixel apart at 30% against 26%. The
16 sites carry 12 distinct stop pairs.

**A quarter, not an eighth.** The ten symmetric sites fade 10, 12, 22, 24, 26, 26, 26, 30, 30, 30 —
median 26%. §5's "roughly the top and bottom eighth" (12.5%) describes two of the ten. The
`10–30% in` envelope, by contrast, is exactly right and is what `seamStops()` clamps into; `70–90%
out` is right for the symmetric shape and does not cover the cut shape's 100%.

So `SEAM_SITES` is the authority for anything reproducing a frame, and `seamStops()` is a documented
fallback for heights nobody drew. Unlike `strokeForSize` (§21) it does not throw: a stop 4% out is
invisible where a stroke 0.5px out is not, and a seam's height is frequently flex-derived at runtime
in a way a hexagon's size never is. Its `height` argument earns its place only through `fadePx`,
converting an intent of "about 40px of fade" into the percentage CSS actually needs.

## 35. The second seam colour had no token — `--seam-panel`

§17 recorded that `2a Compact syncing` draws `#23262D` where every full-window seam draws `#2A2E36`,
and concluded the helper "needs a colour parameter". It does — but it also needs a **token**, and
neither existing one works, because §17's "both map to `#D9D5CE` in light" is a statement about the
_measurement_, not about `tokens.css`:

|                              | dark      | light         |
| ---------------------------- | --------- | ------------- |
| `--seam`                     | `#2A2E36` | `#D9D5CE`     |
| `--border`                   | `#23262D` | `#E6E3DE`     |
| **the panel seam, as drawn** | `#23262D` | **`#D9D5CE`** |

`var(--border)` is right in dark and wrong in light; `var(--seam)` is the reverse. The two seam
colours part in dark and meet again in light, so F3 adds `--seam-panel` (`#23262d` / `#d9d5ce`).
This is the §8 pattern with the themes swapped: there, tokens sharing a dark value diverge in light.

Three sites use it: the 360px compact panel, the tray panel, and — the surprise — `5a Checking`, a
522×766 _window_. The 602×542 `9a First sync` window keeps `var(--seam)`, so this is not a
width rule. All three also happen to be the only 30/70 seams in the design; recorded as an
observation, not a rule, on four samples.

## 36. Rule 3 is missing its load-bearing half: the mask must be POSITIONED

§5 rule 3: _"Centred text and centred buttons that sit on the seam get `background:<surface>` plus
`padding:0 14–18px` so the line passes behind them. `z-index` alone is not enough — the line would
still show between glyphs."_ The warning is about the wrong hazard. The seam is
`position:absolute`, so it paints with the positioned descendants — CSS 2.1 Appendix E step 8 —
**above** both the background (step 4) and the text (step 7) of any static sibling, however late in
the DOM that sibling sits. Background and padding on a static element do nothing.

Confirmed two ways. An isolated page with one absolute 1px line and three following siblings: the
`position:static` block and the inline `<span>` are both painted over; the `position:relative` block
wins. And the frames themselves are a natural experiment — **all 34 in-scope masks are positioned**,
17 on the node itself and 17 inside a `position:relative` wrapper at depth 1, and every one of them
hides the line under a with/without-seam pixel diff. The single drawn mask that is neither is
`1a Compact`, a round-one frame, and it ships the bug: the seam runs visibly between "Syncing" and
"3" in its headline. Applying `seamMask()` to that node makes its seam column pixel-identical to the
same shot with the seam deleted.

`maskStyle()` therefore emits `position:relative` by default, with `position:false` for the wrapper
form. **F8 must generate mask fixtures per site**, not from one expected shape: the split is 17–17,
so neither form alone reproduces the frames — the same trap as §27's `fill="none"`, where the
visually identical node fails the style gate.

A mask can fail two ways, and `auditSeams()` reports both. It can paint underneath (the above), or
it can fail to cover: a background at less than full opacity leaves the hairline showing at reduced
strength, and `--decision-band-bg` is `rgba(255, 107, 107, .05)`, a real token that hides nothing. The
subject of the check is anything _claiming_ to mask — an element with a background of any opacity
straddling the line. An element with no background at all is not reported, because otherwise every
centred flex wrapper in the design is a violation and a check that cries wolf is a check nobody
runs.

## 37. Mask padding: three side values, two vertical, no function of font-size

`padding:0 14–18px` is right about the range and wrong that vertical is always 0. Of the 34 in-scope
masks, 27 are text blocks the helper is for; side padding across them is 14 (13 nodes), 16 (7) or
18 (7), and vertical is `0` on 15 (headlines) and `2px` on 12 (the small mono sub-labels). The
headline tiers are clean — ≥28px→18, 17–22px→16, ≤15px→14 — but four sub-labels take their block's
padding instead of their own tier (`9a Review` 14px→18, `5a Plan safe` 13.5px→18, `2a Syncing` and
`12a Syncing light` 13px→16), so no function of font-size reproduces the set. It is a property of
the centred block, chosen by eye: `maskStyle()` takes `pad`/`padY` with a mid-band default of 16/0
and the screens quote their frame.

The other seven are not text. Six are buttons, which mask with their own fill and their own padding:
`2a Syncing`'s `Pause` uses `--panel-raised` and `11px 22px`, `5a Checking`'s `Stop` the surface and
`9px 18px`. Pass `surface:null, pad:null` and let the control's own styling do the masking — all it
is missing is the positioning. The seventh is `9a First sync`'s 3px progress bar, which masks purely
by being an opaque 400px box on the centre line, with no padding at all.

## 38. The 320ms transition and its reduced-motion behaviour have no frame

§7 lists `320ms ease-out` "for the seam and its columns", and the prototype contains no seam
animation or transition at all — nor any `prefers-reduced-motion` rule anywhere (§30). Prose is
normative on its own here, the same footing as §30. Two decisions recorded rather than measured:

- **Only `opacity` transitions.** A seam's geometry moves whenever the block it spans resizes, and
  transitioning `top`/`bottom`/`height` would drag the line across the content for a third of a
  second every time a list gains a row.
- **Reduced motion drops the transition, never the seam.** Rule 1 makes _presence_ carry meaning, so
  suppressing the element under that preference would remove information rather than movement — the
  same reasoning that keeps the paused hexagon's dasharray in §30.

`setSeamVisible()` leaves the node in the DOM at `opacity:0` rather than removing it: a removed node
cannot transition, and the screen owns when — or whether — to take it out afterwards.

## 39. Two things that look like the seam and are not

- **`--seam-gradient` is one shape of three and one stop pair of twelve.** F1's token is the
  symmetric 26/74 form, which is right for `2a Syncing`, `8a Settings` and `12a Syncing light` and
  wrong everywhere else. `seamGradient()` returns the token for exactly that case, so the most
  common form keeps a single retune point, and composes the other eleven from `--surface`, `--seam`
  and `--seam-panel`.
- **The conflict diff gutter is not this component.** `3a Conflict diff` builds it from
  `grid-template-columns:1fr 1px 1fr` over a `#0D0E11` panel — a flat 1px column showing the panel
  behind it, with no gradient and no fade. `06-plan.md`'s and IMPLEMENTATION-PLAN §5's phrasing
  ("turns the seam into the diff gutter") is about meaning, not construction. S2 should not reach
  for `renderSeam()` there.

`gui/src/styles/seam.css` is a second new stylesheet §3.4 does not list, for the reason §31 gives
for `hexagon.css`: a media query cannot be inline. It needs its own `<link>` in `index.html`, and
omitting that is the same silent failure — the seam snaps instead of fading, with nothing for lint
or CI to catch.

## The shell (F4)

Measured the same way as the seam: the prototype rendered and read off `getBoundingClientRect`, over
the 22 in-scope frames drawn at 1040 (the compacts and the two desktop mocks are F6's and S8's). A
52px header is a number you can read out of the source, but _"does this screen have a footer nav"_ is
a fact about the tree, and the padding that separates four otherwise-identical footers is layout.

The shell is verified rather than eyeballed: `gui/src/index.html` is loaded in a browser on the
mock-data path and the chrome it builds is asserted against these numbers — 27 checks covering the
header, the chip, the doors, the door/action-bar switch and the home affordance.

## 40. The four doors are not on every screen

`02-shell.md` §"Footer navigation": _"These four never move and never change order, on any screen, in
any state."_ True about order, and it reads as _present everywhere_. Measured, every in-scope 1040
frame carries **either** the four doors **or** a footer action bar — never both, never neither:

|                       | Frames                                                                                    |
| --------------------- | ----------------------------------------------------------------------------------------- |
| four doors (13)       | `2a` ×3 · `3a` ×2 · `4a` ×2 · `6a Activity passes` · `7a` ×2 · `12a` ×4                   |
| footer action bar (6) | `5a Plan` · `5a Plan safe` · `8a Settings` · `8a Skip rules` · `9a Folders` · `9a Review` |

The action bar **replaces** the doors on the screens that commit something. That is coherent — you do
not wander off mid-commit — but it has a consequence the docs do not draw: **Settings, Plan and
onboarding have no navigation at all.** The only ways out are the action bar's own secondary button
and the app mark.

Which settles half of `IMPLEMENTATION-PLAN.md` §3.3's open question by elimination. The plan
_assumes_ "clicking the active door returns to root, and the app mark is also a home affordance"; for
Activity the first works, but on Settings and Plan there is no door to click, so **the app mark is
not optional**. `routes.js` carries a `footer` field per route for this, and `renderHeader` takes an
`onHome`.

## 41. The footer nav has four padding variants, not a range (§1.3 conflict 7, closed)

`0 40px 18–22px` with `padding-top:14–20px`. All four combinations are drawn, so — like the seam's
`-114`/`-150` (§33a) — it is a table, not a range to pick from:

| bottom / top | Mono line | Height | Frames                                                                    |
| ------------ | --------- | ------ | ------------------------------------------------------------------------- |
| 22 / 20      | yes       | 89     | `2a Settled` · `2a Syncing` + both light twins                            |
| 20 / 16      | no        | 53     | `2a Needs you`                                                            |
| 18 / 15      | no        | 50     | `3a` ×2 · `4a` ×2 · `6a Activity passes` · `12a Deletions/Conflict light` |
| 18 / 14      | no        | 49     | `7a Activity quiet` · `7a File lookup`                                    |

§1.3 conflict 7 names two of the four (`2a` 22/20, `7a` 18/14). The **majority** variant, 18/15 across
six frames, is in neither the conflict note nor `02-shell.md`. The mono line appears on exactly the
two frames with the widest padding, which is what the extra 4px is for.

Everything else is invariant across all thirteen: `gap:34px`, centred, `border-top:1px #16181D`, doors
at 13px/**400** (§11's finding again — the prose never names a weight and 400 is what is drawn),
`#828B98` inactive and `#F2F4F7` active. No light frame draws an _active_ door, so the light active
colour (`#14161A`, from the doc) joins the unverified list in §15.

## 42. The header dims off the status chip, not off the screen

§1's table gives the mark `opacity:.65–.75` "on settled/secondary screens" and `1` "when something is
happening", and the product name `#99A2AE` when settled. "Settled/secondary" is a judgement to be made
per screen. Measured, it is a predicate: **`chip === "idle"`**. All six idle frames dim both the mark
and the name; all fourteen non-idle frames dim neither — including `5a Plan`, whose chip is
`rehearsal`, and `9a Folders`, whose chip is `step 1 of 2`.

And `.65–.75` is not a range either: it is **`0.75` in dark and `0.65` in light**. One light settled
frame exists (`12a Settled light`), so this rests on a single sample; `--app-mark-quiet` carries it
and it belongs on §15's unverified list until S10 confirms.

The home affordance costs one node the frames do not draw: the mark becomes a `<button>` so it is
keyboard-reachable, sized to exactly 20×20 with no padding so the header's 12px gaps do not move.

## 43. The status chip — six variants, a 1px ring, and a token the palette lacked

Five variants in §2's table, all confirmed with one correction, plus a sixth the doc describes in prose
and leaves out of the table.

| Variant               | Border                        | Text              | Dot                        |
| --------------------- | ----------------------------- | ----------------- | -------------------------- |
| idle                  | none                          | `--text-label`    | `--dot-inert`, 6px         |
| syncing               | 1px `--border-chrome`         | `--text-3`        | `--up-to` + `blip 1.6s`    |
| n waiting (decisions) | 1px `--chip-attention-border` | `--decision-text` | **1px** ring `--decision`  |
| n waiting (deletions) | 1px `--chip-attention-border` | `--decision-text` | filled `--destructive`     |
| rehearsal             | 1px `--border-chrome`         | `--text-3`        | none                       |
| **step N of 2**       | none                          | `--text-5`        | none — **and no ⋯ button** |

- **The ring is 1px, not the 2px §2 states** — in both themes.
- **Onboarding drops the ⋯ button too.** §2 says only that the chip is "omitted on onboarding
  (replaced by `step 1 of 2`)". Both `9a` frames have **four** header slots, not five. A menu whose
  only item is the theme toggle has no place in a two-step flow that cannot be left.
- **The border needed a new token.** Measured `rgba(255,107,107,.35)` dark → `rgba(190,18,60,.30)`
  light, against the then-`--decision-border`'s `.32`/`.28`. Close enough to look like the same value
  and not it; the alphas differ per theme, so no `rgba(var(--decision-rgb), …)` form covers both.
  Added as `--chip-attention-border`. **§52 later found that the `.32`/`.28` compared against here
  was not one token's theme pair at all** — it was two different bands, one measured per theme.
- The syncing dot measures 8.8px rather than 6px only because `blip` scales it 1.5× and the reading
  caught it mid-cycle. It is a 6px dot in every variant that has one.
- `blip` lands in `shell.css`, which is where §29 said it belonged: it drives this dot and (F5) a text
  caret, and never a hexagon.

## 44. Chip priority: measured where the frames settle it, chosen where they do not

`2a Needs you` is syncing **and** has three decisions waiting, and its chip reads `3 waiting` — so a
decision outranks a transfer. That is the only ordering the frames settle. Two are chosen:

- **Deletions over decisions.** Nothing draws both at once. Deletions win because that is the one that
  ends with a file gone.
- **`paused`, `unreachable` and `authExpired` have no drawn chip anywhere.** They take the quiet form
  with their own text rather than an invented colour — the hexagon and the main screen carry those
  states, which is where the design puts them.

## 45. `Ctrl Q` — the design and the shipped app disagree about what Quit does. **Open.**

`14-behaviour-and-state.md` and `10-tray.md` are explicit and consistent with each other: `Ctrl W`
closes the window _keeps syncing_, `Ctrl Q` quits _stops syncing_, and the tray must carry those as
sub-labels because _"this is the single worst misunderstanding a tray app can cause"_.

The shipped `tray.rs` does something else, deliberately and with a comment: `quit` is `app.exit(0)`,
ending the GUI process while **the daemon keeps running**, "a separate process and unaffected by
either" (v1 design §3.7, refined in #88).

F4 wires `Ctrl Q` to the **existing** path, so the shortcut and the tray item beside it cannot mean
different things, and does not resolve the contradiction. Stopping the daemon is a lifecycle decision
with data consequences; it belongs to S8 (#187), which owns the tray, and guessing the more
destructive of two readings from a keyboard shortcut is the wrong way to settle it.

## 45a. The shell is built once and patched, never rebuilt on the poll

Not a deviation from the design — a constraint the design already stated, which the first version of
`app.js` broke. `updateHexagon`'s comment (§F2) says the screens "must call this rather than
re-rendering", because `replaceChildren` restarts both CSS animations from 0%; that was written
about the hexagon, and it is a shell problem before it is ever a screen problem.

Measured on the first draft: the shell re-rendered on every status poll (~2 s), and tabbing to a
door dropped keyboard focus to `<body>` **within 1.2 seconds**. `14-behaviour-and-state.md` requires
every control to be keyboard-reachable "because this is a desktop app", and a window that discards
focus twice a second is not one.

So `render()` patches: `updateHeader` and `updateFooterNav` return `false` when the change is
structural (the ⋯ appearing, a different footer variant) and the caller rebuilds only then, and the
body is replaced only when the route changes. The status chip's node is replaced only when its
_variant_ changes, so a count ticking 2 → 3 does not restart the `blip` on its dot. Asserted: focus
and node identity both survive two poll ticks.

This is a constraint on S1–S11 as much as on F4 — a screen that rebuilds its own subtree on every
poll reintroduces it locally.

## 46. Two more things nothing draws, and one the frames disagree with

- **The ⋯ menu is never drawn open.** `02-shell.md` gives it one sentence — the theme toggle moves
  there from the title bar — and that is the whole specification. Same footing as §30 and §38.
- **The content region's `overflow`.** §2 calls this "a real bug found twice during this design" and
  offers two fixes: `overflow:hidden`, or size the content to fit. The frames take the second — 21 of
  22 leave it `visible`, and only `6a Activity passes` sets `hidden`. `.content-region` takes the
  **first**, because sizing-to-fit is a property of a fixed dataset and the app's data is not fixed:
  one long filename is all it takes. Clipping is the recoverable failure; painting over the footer is
  not.
- **Content `margin-top` is not `22–26px`.** Measured across the frames: 14, 18, 20, 22, 24, 26, 30,
  34 and 36. It is per-screen, so the shell does not set it and each screen carries its own.

## The fidelity harness and the copy deck (F7, F8)

## 47. The harness renders the prototype; it does not parse it

`IMPLEMENTATION-PLAN.md` §1.4 and the F8 issue both describe `extract.mjs` as parsing
`Drive Sync.dc.html` and emitting "inline styles expanded to longhand". It renders it instead, in a
real engine, and that is what makes the output comparable at all.

`assert.mjs` reads the app through `getComputedStyle`. If the prototype's side came from a parse, the
two would not be the same kind of thing: a parse yields `padding:0 20px` where the app yields four
longhand pixel values, and every cascade, inheritance, UA default and shorthand expansion is missing.
Expanding shorthands by hand would reimplement a CSS engine badly.

Same reason F3 rendered the prototype to measure the seam, and F4 to measure the shell: what is being
compared is what an engine computes, so an engine has to compute both sides.

## 48. Every framed surface is drawn 2px larger than it is declared

`DEVIATIONS.md` §19 recorded this for the 360px compact panel and told F6 to write 362. It is
general. The prototype does not opt these nodes into `border-box`, so the 1px border sits outside the
declared box:

| Surface           | Declared   | Drawn      |
| ----------------- | ---------- | ---------- |
| the window        | `1040×764` | `1042×766` |
| `5a Checking`     | `520×764`  | `522×766`  |
| `6a Details`      | `520×460`  | `522×462`  |
| `9a First sync`   | `600×540`  | `602×542`  |
| the compact panel | `360×296`  | `362×298`  |

`base.css` sets `box-sizing:border-box` globally, so an app surface written at its nominal size comes
out **2px narrower than the frame**. F5's dialog layer and F6's panel both have to write the drawn
number, or set `content-box` on that one element. `IMPLEMENTATION-PLAN.md` §3.3's "Details opens a
520×460 dialog" is the declared size, not the drawn one.

The harness sidesteps the whole question rather than encoding an offset: **size is compared as a
border box** (`getBoundingClientRect`, identical in both box models) and the computed `width`/`height`
properties are not asserted at all. Comparing those would measure the box model, not the design — the
same element reports `1000px` in one document and `1040px` in the other while occupying identical
space on screen.

### 48a. "+2px" is a consequence, not a rule — four of the ten dialogs opt in

F5's dialog layer needed a number to write, so the ten `kind: dialog` frames were read back against
the prototype rather than offset by two. **Four of them declare `box-sizing:border-box` inline**, and
those come out at exactly their nominal size:

| declares `border-box`                                                | declared = drawn |
| -------------------------------------------------------------------- | ---------------- |
| `9a Consent`, `9a CLI missing`, `8a Save refused`, `7a File pending` | `600`            |

| does not                                                        | declared → drawn |
| --------------------------------------------------------------- | ---------------- |
| `3a Conflicts cleared`, `5a Checking`, `4a Empty`, `6a Details` | `520` → **522**  |
| `9a First sync`, `7a Never synced`                              | `600` → **602**  |

The split is not arbitrary: the four that opt in are the four that carry `padding` on the dialog
itself, and the six that do not are `display:flex` columns with a fixed height that pad their
children instead. An author reaching for padding reached for `border-box` in the same breath.

So **there is no offset to apply — only a number to read off the frame**, and `routes.js` carries the
drawn box per dialog with a test asserting it. Under `base.css`'s global `border-box`, writing the
drawn number is always right; writing the declared one is right four times out of ten.

`3a Conflicts cleared` and `5a Checking` are worth naming separately. Both are `522×766` and both
contain **the app header and the four footer doors**, so they are not 520px dialogs at all — they are
the product window drawn narrow, because their content is a centred empty state and 1040px of it
would be mostly whitespace. Neither is modelled by the dialog layer.

## 49. The copy deck outlived one of its drawings

`13-copy-deck.md`'s Activity section carries `Nothing has moved in the last hour.` under "Quiet:".
Its only frame is `6a Quiet`, one of the two demoted tide-chart Activity frames that
`IMPLEMENTATION-PLAN.md` §1.2 puts out of scope — so the deck specifies a string the in-scope design
never draws.

It stays in `ui/copy.js`, because `14-behaviour-and-state.md`'s empty-state table still specifies it
("Activity › files: `Nothing has moved in the last hour.` + flat line") and S5 will need it. The copy
gate exempts it by name with that reason, which is the point of having the exemption list be explicit
rather than a threshold.

### 49a. Two things the copy gate had to learn to see

Neither is a copy error; both would have silently exempted whole categories of string.

- **Sentences are split by inline children.** The deck has 25 `<strong>` elements mid-paragraph, so
  `"Yours has buy milk where Proton's has..."` is a node whose own text is `"Yours has where
Proton's has..."` plus a child. The fixtures now record the joined subtree text alongside each
  node's own — the style gate needs to know which node holds the words, the copy gate needs the
  sentence.
- **A placeholder is copy.** `"Add a rule — e.g. *.psd or scratch/**"` appears in no `textContent`
  anywhere in the prototype. The fixtures now record `placeholder`, `value`, `title`, `aria-label`
  and `alt`; without them every input in the design is exempt from the gate for free.

## 50. What the first live run of the style gate found

Recorded because it is the argument for building the harness before the screens rather than after.
Six classes of drift in F4's shell, all fixed in the same commit, none of them visible by eye:

| Drift                                                                                         | Why it survived review                                                                                 |
| --------------------------------------------------------------------------------------------- | ------------------------------------------------------------------------------------------------------ |
| the app mark carried `width`/`height` **attributes** the frames do not set                    | identical rendering; §27's redundant-`fill` trap in a new place                                        |
| `.chip`, `.chip-dot` and `.menu-btn` used `flex:none` where the frames leave `flex-shrink:1`  | all three have fixed widths, so nothing ever shrinks them                                              |
| `.door` had a `<button>`'s UA `text-align:center` where the frames' `<span>`s inherit `start` | the bar centres them with `justify-content` regardless                                                 |
| the footer used the `withLine` variant on every main screen                                   | `2a Needs you` drops the mono line and tightens 22/20 → 20/16 — the attention band has taken the space |

The last one is a real bug, not a technicality, and it is the kind that would have been copied into
S1 and every screenshot after it.

## 51. What the harness cannot cover, stated rather than implied

A gate that appears to cover more than it does is worse than one that admits its edges, so this list
lives in `gui/tools/fidelity/README.md` as well:

- **The seven screens with no drawn light frame.** S10 asserts those against `12-light-theme.md`'s
  mapping table, which is prose, not a drawn artefact — the same standing already given to §15's
  unverified light values.
- **Whether an animation looks right.** Only the declaration is comparable. A wrong easing that
  parses is invisible, as is anything about §30's open reduced-motion question.
- **Native tray rendering** and **the desktop's own notification chrome.** Neither is a webview.
- **Motion, focus order and hover states.** The gate reads a static tree. F4's focus-survival check
  and the seam audit cover their own corners of this; nothing covers it in general. §55a is what
  that gap costs: a control can satisfy every asserted property and still be impossible to operate,
  and the typed-`DELETE` gate did.

The engine is Chromium rather than the WebKit the issue asks for, because Playwright's WebKit needs
host packages this machine cannot install without `sudo`. It is the same engine every measurement in
§8–§50 came from, so the numbers here and the numbers there are consistent — but WebKitGTK is what
ships, and CI should add it when the runner allows.

---

## Controls, rows and bands (F5)

## 52. `--decision-bg`/`--decision-border` were one token pair holding two different bands

Found by censusing the band tints before writing `bands.js`, not by looking at the app — the defect
is invisible in either theme on its own.

Sweeping all 51 frames for a translucent fill over a translucent border returns **ten** crimson,
red and amber band surfaces. Two of them are relevant here:

| site                                                         | dark                            | light                                                 |
| ------------------------------------------------------------ | ------------------------------- | ----------------------------------------------------- |
| attention band — `2a Needs you`                              | `rgba(255,107,107,.05)` / `.32` | **not drawn**                                         |
| recoverable card — `4a Deletions` right column, `9a Consent` | `rgba(255,107,107,.04)` / `.30` | `rgba(190,18,60,.03)` / `.28` (`12a Deletions light`) |

F1 carried one pair: `--decision-bg`/`--decision-border`, dark `.05`/`.32` and light `.03`/`.28`.
Those are **the attention band in dark and the recoverable card in light** — two different surfaces,
one token. It could not have been otherwise: the light frame set is settled / syncing / compact ×3 /
conflict / deletions / tray, and **no light frame draws the attention band**, so the light value had
to come from somewhere else.

Nothing consumed the pair yet, so nothing rendered wrong. It would have gone wrong the moment
`bands.js` used it — whichever band claimed it, one theme would have drawn the other band's tint,
and no screenshot review of a single theme could catch that.

**Resolution.** Split by site, the same move §43 made for `--chip-attention-border`:

| token                    | dark                    | light                              |
| ------------------------ | ----------------------- | ---------------------------------- |
| `--decision-band-bg`     | `rgba(255,107,107,.05)` | `rgba(190,18,60,.04)` — **chosen** |
| `--decision-band-border` | `rgba(255,107,107,.32)` | `rgba(190,18,60,.30)` — **chosen** |
| `--decision-card-bg`     | `rgba(255,107,107,.04)` | `rgba(190,18,60,.03)`              |
| `--decision-card-border` | `rgba(255,107,107,.30)` | `rgba(190,18,60,.28)`              |

Three of the four light values are measured. The light attention band is **chosen**, because nothing
draws it. Derived from the card, which is drawn in both themes: `.04`/`.30` dark → `.03`/`.28` light,
so the band's `.05`/`.32` → `.04`/`.30`. The cross-theme ratio (×0.75 on the fill) and the
band-to-card delta (−`.01`/−`.02`) independently give the same two numbers, which is the only
corroboration available without a frame. **S10 owns confirming it** when the light mapping is
propagated to the undrawn screens.

The `--destructive-*` pair was checked for the same defect and is clean: dark `.06`/`.38` and light
`.04`/`.40` both come from `4a Deletions`' permanent card and its light twin. One token pair was
broken, not the family.

### 52a. The rest of the band tints, for whoever writes `bands.js`

The other eight, unresolved here because they want measuring in place rather than up front:

| site                                | fill                     | border |
| ----------------------------------- | ------------------------ | ------ |
| `4a Deletions` permanent, `5a Plan` | `rgba(255,59,59,.06)`    | `.38`  |
| `4a Compact` permanent              | `rgba(255,59,59,.05)`    | `.32`  |
| `4a Compact` recoverable            | `rgba(255,107,107,.035)` | `.26`  |
| `8a Deletions tab`                  | `rgba(255,59,59,.04)`    | `.30`  |
| `11a Rules`                         | `rgba(255,59,59,.05)`    | `.30`  |
| `7a Activity quiet` never-synced    | `rgba(255,159,28,.04)`   | `.28`  |

Two things fall out. **The compact panel steps every alpha down one notch** (crimson `.04`→`.035`,
`.30`→`.26`; red `.06`→`.05`, `.38`→`.32`), so tone and density are independent axes — the same
shape `controls.js` found for kind and size. And **the never-synced band's amber has no token in
either theme**: `--up-label` is the same hue in dark (`#ff9f1c`) but `#b23f14` in light, a different
colour entirely. No light frame draws that band either, so it lands in the same chosen-value
position as the attention band above.

No band carries a solid fill in any frame. The one solid red in the app remains the
`Delete permanently` button.

## 53. The transfer row's arrow is beside the seam, not on the outside edge

`03-main-screen.md` §"Transfer row":

> Left column: filename → size → `→` `#FF9F1C`. Right column: `←` `#22D3EE` first, then filename,
> then size. **The arrow is on the outside edge in both columns**, pointing away from the seam.

The first sentence is correct and the second contradicts it. `div[1]` on `2a Syncing` is
`grid-template-columns: 488px 488px`, so `div[0]` is the left, leaving column. Measured child order:

| column           | order               | where the arrow lands              |
| ---------------- | ------------------- | ---------------------------------- |
| left — leaving   | `[name] [size] [→]` | the right end, **beside the seam** |
| right — arriving | `[←] [name] [size]` | the left end, **beside the seam**  |

Both arrows sit against the centre line and both point across it, which is also the direction of
travel. "Outside edge" and "pointing away from the seam" are wrong on both counts.

**Checked against the light pair rather than assumed.** `12a Syncing light` (`div[1]` likewise
`488px 488px`) and `12a Compact syncing light` carry the identical order. All four drawn frames
agree — this is prose against frames, never frame against frame, so §1.3 settles it without a
judgement call.

Issue #169 repeats the wrong gloss verbatim, so the sentence has already propagated once. `rows.js`
carries the rule in a pure `transferSlotOrder()` with its own test, because a screen built from the
prose still looks plausible and the style gate cannot catch it until S1 maps `2a Syncing`.

Two details the correct sentence also gives, worth keeping together with it:

- **It is a rotation, not a mirror.** `[name][size][arrow]` becomes `[arrow][name][size]`; the name
  precedes the size on both sides. A mirror would put the size first on the right, and nothing
  draws that.
- **Placement follows direction, not column.** `2a Compact syncing` is a single 360px column with
  no seam and no second side, and its arriving row still leads with the arrow.

### 53a. Three smaller things the row census settled

- **The flat rows are one shape at several rungs, and F5 models five of them.** history, path,
  fact, pass and action are all `padding:<y> 2px` over a 1px `--divider` rule, flex, centred:
  `9/13`, `9/12`, `11/14`, `12/14`, `13/14` (y / gap). Written once in `rows.css` as five rungs
  rather than five builders.

  **These five are not the whole set, and the ladder is deliberately open.** Censused over the
  prototype, `padding:<y> 2px` beside `#16181D` also yields an `8px 2px` gap-13 rung — and that one
  puts the rule on the **bottom**, not the top. `5a Plan` draws nine of them (`div[3]/div[1]/div[0..8]`,
  976×33); eleven `border-bottom` separators across the prototype in all, against ninety-two
  `border-top`. That is S4's screen, not F5's scope, so F5 does not model it; a screen that needs a
  rung adds one. Stated because "four rungs" written as though exhaustive is the same failure mode
  as §53's arrow gloss — a sentence the next person trusts instead of re-measuring.

- **`7a Never synced`'s entries are not fact rows**, though they sit one screen away and look like
  them. No dot, `gap:12` against `14`, and a **mono** path rather than a sans sentence — plus a
  dimmed variant that separates _you told it to skip these_ from _can't be synced_. Modelled as
  `pathRow` rather than stretched onto `factRow`. Its `Change this rule` is likewise **not** an
  action row: a standalone `inline-block` button after the group, at controls.js's plain `small`
  size (`7px 14px`, `--r-8`).

- **A sub-pixel border in the prototype that never reaches the pixels.** `5a Plan`'s conflict dot is
  authored `border:1.5px solid #FF6B6B`, but Chromium floors a sub-pixel border at `deviceScaleFactor:1`
  and the frame records **1px** — there is not a single `1.5px` border in any of the 51 fixtures.
  So the decision dot's ring is 2px at 7 and 8 and 1px at 6 everywhere that is drawn, and `dot()`
  defaults off size accordingly. Recorded because reading the prototype and "restoring" the 1.5px
  changes nothing on screen while making the source disagree with the ground truth. The one real
  constraint is that a 2px ring on a 6px dot leaves a 2px hole and reads as a fill.
- **`02-shell.md` §2's transfer row is the compact one.** It gives `border-radius:9px;
padding:9px 11px`, against `03-main-screen.md`'s `11px` / `11px 13px`. Not a conflict: that
  section describes the 360px panel (its hexagon is 72px), and the frames measure exactly those two
  rungs — 11/11×13 on `2a Syncing`, 9/9×11 on `2a Compact syncing`. `--r-11`'s "transfer rows"
  comment is right for the main screen and needs the compact rung read off `--r-9`.
- **The deletion card's headline is 16px sans — the only filename in the app that is not mono.**
  Measured on both cards of `4a Deletions`. Deliberate rather than a slip: the card is naming the
  thing you are about to lose, and the path itself is in the facts strip below in mono.

`4a Compact`'s two deletion rows (332×61, no facts strip, no gate, no second button) are **not**
this card at a smaller size and are left to F6 with the rest of the panel. They also need the two
compact band alphas §52a records, which no token carries yet.

## 54. The never-synced band's amber had no token in either theme

§52a flagged it; `bands.js` needed it. Measured `rgba(255,159,28,.04)` over `rgba(255,159,28,.28)`
at `7a Activity quiet`, radius `13`, padding `15px 18px` — the only site.

The nearest existing token was `--up-label`, which carries the same `#FF9F1C` in dark and a quite
different `#B23F14` in light. Reusing it would have worked in dark and made the band's meaning
depend on a token named for upload accents, so **`--warn` is a new token with the same value in both
themes and deliberately not an alias**. `tokens.css` already keeps several same-valued pairs apart
for exactly this reason — `--border` / `--border-chrome`, `--panel-alt` / `--compact-row` — so that
either can move in light without dragging the other.

| token                | dark                   | light                              |
| -------------------- | ---------------------- | ---------------------------------- |
| `--warn`             | `#FF9F1C`              | `#B23F14`                          |
| `--warn-band-bg`     | `rgba(255,159,28,.04)` | `rgba(178,63,20,.03)` — **chosen** |
| `--warn-band-border` | `rgba(255,159,28,.28)` | `rgba(178,63,20,.28)` — **chosen** |

Both light tints are chosen, because no light frame draws this band. Derived as §52 derived the
light attention band — from a drawn pair starting at the same dark alpha. The recoverable card is
`.04`/`.30` dark and `.03`/`.28` light, so a `.04`/`.28` dark band maps to `.03`/`.28`. **S10 owns
confirming it.**

**Why warm and not crimson, restated because it is the band most likely to be "corrected":** nothing
in it is at risk. The screen's own closing line is _"Nothing here is at risk — it's just not backed
up."_ Four skipped temp files inside a crimson band would be a lie about severity, and the amber is
the design saying so.

### 54a. What the band census settled about layout

- **The attention band is ONE box, not one per item.** `2a Needs you` is a single 976×127 container
  holding both waiting items, split by a 1px `--decision-divider` rule, `overflow:hidden` so the
  rules do not cross the 14px radius. Two conflicts and a deletion queue are one interruption.
- **Every value moves with tone, not with size.** The destructive band and the never-synced band
  differ in padding (`16px 20px` / `15px 18px`), radius (`14` / `13`), gap (`16` / `12`), title size
  (`14` / `13.5`), note colour (`--text-2` / `--text-3`) and title colour (`--destructive-text` /
  plain `--text`). Tone is the axis, the way kind is in `controls.js`.
- **No band holds a destructive action, and none is ever a solid fill.** `11a Rules` states the
  first for banners; the four bands here keep it too — `Review`, `Compare`, `Show them`, `Leave it
alone` all route to the screen that owns the decision.
- **`8a Deletions tab` and `11a Rules` are tinted like bands and are not bands.** The first is
  `controls.js`'s `radioCard` wearing a destructive tone (546×92.75, a 15px ring at `#6B3A3A` — a
  ring colour no token carries, S6's to mint); the second is a prose callout on a spec sheet, with
  no glyph and no button. Neither is modelled by `noticeBand`.

## 55. The one drawn checkbox disagreed with `controls.css` in six places

`9a Consent`'s `I understand deletions travel both ways.` is the **only** checkbox in any of the 51
frames — swept for 17×17 nodes at radius 5, exactly one hit. F5's controls commit wrote the block
from the general control conventions instead, and nothing caught it until `bands.js` built the
consent panel around it and the panel came out **2.5px short**.

| property            | was                         | drawn                |
| ------------------- | --------------------------- | -------------------- |
| box border          | `--border-strong` `#2E323A` | `--text-5` `#6D7783` |
| box background      | `--panel-alt`               | transparent          |
| box `margin-top`    | —                           | `1px`                |
| row `gap`           | `10px`                      | `11px`               |
| label colour        | `--text-2`                  | `--text-bright`      |
| label `line-height` | `normal`                    | `1.5`                |

Two of those are load-bearing rather than cosmetic. The box is **transparent** because it sits on the
consent panel's crimson tint and a `--panel-alt` fill punches a dark hole in it. And the border is
**`--text-5`, not `--line-inert`** — `--line-inert` (`#3E454E`) is the _unselected radio ring_, and a
checkbox you have not ticked is drawn brighter than an unselected radio, because it is the thing
standing between you and continuing.

The same control also carried a **second, stale source of truth**: an `is-checked` class stamped on
the box from the `checked` argument at build time. Nothing consumed it — the visual state comes from
`.checkbox-input:checked + .checkbox-box`, which the browser keeps current for free — so nothing was
visibly wrong. It was a trap for whoever styled it next: a class that reads like state and freezes at
its initial value the moment anyone ticks the box. Removed.

**`toggle`'s `is-on` and `radioCard`'s `is-selected` are not the same thing and must stay.** Neither
has an `<input>` beneath it — a toggle is a `<button role="switch">`, a radio card a
`<div role="radio">` — so there the class _is_ the state and the caller re-renders to change it. The
distinction is whether the DOM already knows; the checkbox is the only one of the three where it
does. Written into the builder's own docstring, because "consistency" is exactly the argument that
would put `is-checked` back.

The `line-height` is what the missing 2.5px was: `normal` closes the leading on a sentence that
wraps. Recorded because a 2.5px height error is invisible by eye, sits inside the style gate's
tolerance for nothing else, and would have been attributed to S7 when it landed.

### 55a. "Clears on blur" made the typed-`DELETE` gate impossible to complete

`14-behaviour-and-state.md` asks for a field that is case-sensitive, keeps the confirm button
disabled until the word matches, and **clears on blur**. Implemented literally, on the field's own
`blur`, all three hold and the gate cannot be used.

Reaching the button the gate unlocks blurs the field on the way to it. So the field cleared,
`onChange(false)` disabled the button, and the click already in flight landed on a disabled control.
Reproduced headlessly with a consumer that patches the button rather than rebuilding it:

| path                          | before                    | after |
| ----------------------------- | ------------------------- | ----- |
| pointer — click `Delete`      | handler fired **0** times | fires |
| keyboard — `Tab` then `Enter` | handler fired **0** times | fires |

Both, because `mousedown` blurs before `click` dispatches and `Tab` blurs before `Enter` arrives.
The only irreversible action in the app could not be performed at all, by any means.

**The rule is right; the boundary was wrong.** "You went and did something else" is not observable at
the field — moving to the button the field unlocks is the second half of the same act. It is
observable at the PAIR. So `deleteGate` stands down for any blur whose `relatedTarget` is inside
`[data-delete-gate]`, and `deletionGate` stamps that attribute and watches the group's own
`focusout`. Both halves are needed: without the first the gate cannot be completed, and without the
second a gate left armed while you tab past the button stays armed, which is the formality the rule
exists to prevent.

Verified in all four directions — click completes, `Tab`+`Enter` completes, focusing anything outside
the pair clears, and tabbing past the confirm button without pressing it clears.

**No automated regression guard**, and that is a real gap on the app's most safety-critical control.
It is DOM-behavioural, so `node:test` cannot reach it, and the fidelity gate reads a static tree —
§51 already lists focus order among what it cannot cover. The reproduction above is the recipe;
whoever builds S3 should carry it into whatever browser-driven harness that screen earns.

## 56. The scrim behind a dialog has no ground truth anywhere. **Open.**

Every dialog in the bundle is drawn in isolation — its own `data-screen-label`, its own box, on the
spec sheet's background. **Not one frame shows a dialog over a screen**, so the layer between them
is undrawn. Neither `02-shell.md` nor `14-behaviour-and-state.md` specifies it either; the only
mention of dialogs in either is `14-behaviour-and-state.md:116`, "`Esc` cancel a confirmation or
close a dialog".

Chosen rather than left absent, because a modal with no scrim over a live screen is a worse default
than a slightly wrong one — the screen behind stays visually clickable and the modality is invisible.

`--scrim: rgba(0,0,0,.5)` dark, `rgba(0,0,0,.33)` light. Derived from the one related pair that IS
drawn: `--shadow-dialog` steps `.6` dark → `.4` light, and `.5 × (.4/.6) = .33`. Deliberately modest
— the dialog already carries a `0 24px 60px` shadow at `.6`, and a heavy scrim under it draws the
same separation twice.

**A designer should confirm the value.** Unlike §52 and §54, there is no drawn analogue at the same
site to derive from, only a proportion borrowed from a shadow. Filed as open in the same sense as
§45.

## 57. Only three of the seven "overlay" routes are dialogs

F4's route table gave seven routes `kind: "overlay"`, which is correct about routing — all seven
stack over what you were doing, all seven return focus to their opener, all seven answer `Esc`. It
is not a statement about presentation, and F5 needed one.

Measured by what each route's frame actually is:

| route         | frame                       | drawn as                                                        |
| ------------- | --------------------------- | --------------------------------------------------------------- |
| `details`     | `6a Details` `522×462`      | **dialog** — standalone surface, ✕, no header, no doors         |
| `neverSynced` | `7a Never synced` `602×602` | **dialog** — ✕ and a `Done`                                     |
| `saveRefused` | `8a Save refused` `600×213` | **dialog** — no ✕, two actions                                  |
| `conflicts`   | `3a Conflict` `1042×766`    | screen — keeps header and doors                                 |
| `deletions`   | `4a Deletions` `1042×766`   | screen — keeps header and doors                                 |
| `armed`       | `4a Armed` `1042×766`       | screen — keeps header and doors, replaces the content area only |
| `onboarding`  | `9a` ×5                     | takeover — non-dismissible, already special-cased by F4         |

So four of the seven need **no scrim, no focus trap and no layer at all**: the body swap F4 already
does is exactly right for them, and wrapping them would put a scrim over a window that has nothing
behind it. `routes.js` carries `presentation: "dialog" | "screen"` with tests, because the two are
one word apart in the table and the wrong one is not a crash — it is a scrim over a screen that
should have been replaced, or a full window with no way back.

`4a Armed` is the one worth stating outright: **the full-window delete confirmation is a body swap,
not a dialog.** It keeps the live status chip and the four doors, draws `Press Esc to cancel.` on
screen, and is centred on the 104px warning hexagon. It needs the layer least of all seven and would
have been the most tempting to wrap.

### 57a. What the layer had to change in F4, and what it must not touch

**One branch.** `app.js:305` was the only place keyed off the overlay; the header and footer already
read `route`. A dialog now collapses back to its underlying route so the screen beneath renders as
though nothing had opened — including `onMain`, which keyed off `!overlay` and would otherwise swap
the footer's mono line away and grow a home button in the header the moment Details opened. The
shell visibly rearranging behind a panel sitting on top of it is exactly what F4's own note on the
`details` route asks to avoid: _"clicking it must not lose your place."_

**No second `Escape` handler.** F4 owns the key and its precedence chain — menu, then overlay, then
the screen's `shell:cancel`. A listener in the dialog layer would give one keypress two effects.
`dialog.js` handles `Tab` and nothing else.

**The layer is keyed and patched, never rebuilt.** Same discipline as §45a and for a harder reason:
the armed deletion's typed-`DELETE` field clears on blur by design, so a layer rebuilt on the ~2s
poll would destroy the field mid-word and make the gate impossible to finish. Verified by holding a
node reference across two polls.

### 57b. One overlay slot cannot hold two layers — found in review

The first version of the dialog layer kept F4's single `overlay` variable and derived the body from
it: `active = dialogRoute ? route : (overlay ?? route)`. That reads as conservative and is not. With
one slot, opening a dialog **overwrites** whatever screen overlay was showing, and the fallback to
`route` then drops the user onto the door underneath.

It is one click away, not a corner case: **all three screen overlays draw the four doors**, `Details`
among them, measured on `3a Conflict`, `4a Deletions` and `4a Armed`. So opening Details from the
Deletions screen lost the Deletions screen — the exact failure F4's note on the `details` route warns
about, reintroduced by the module written to honour it.

Fixed by giving the two presentations two pieces of state: a **stack** for screen overlays (so
`4a Armed` over `4a Deletions` returns to the deletions screen rather than to a door) and a single
slot for the dialog, which is always above it. `Esc` closes the topmost — dialog first, then the
stack — and each layer carries its own focus-return key. Opening a screen overlay dismisses any
dialog, because the dialog belonged to the screen being left.

Verified end to end: screen overlay open → dialog over it keeps the screen mounted → `Esc` closes
only the dialog and returns focus to the door that opened it → `Esc` again closes the screen overlay.

Two smaller things from the same review, both real:

- **The ✕ is per dialog.** The layer passed `onClose` unconditionally, contradicting this file's own
  §57 table — `8a Save refused` and `9a CLI missing` draw no close button, because they ask you to
  choose between two repairs and a dismiss in the corner is a third answer the design does not offer.
  Now `closable` in `routes.js`, with a test. `Esc` still closes all three.
- **A dialog takes `label` or `labelledBy`, never both and never neither.** ARIA gives
  `aria-labelledby` precedence, so passing both is not an error that surfaces — it is a `label` that
  silently does nothing while its author believes the dialog is named. Both cases now throw.

### 57c. The onboarding takeover hid the layers instead of discarding them

The first version forced `dialogRoute` to `null` and `active` to `"onboarding"` while the latch was
set, which is correct for what is _shown_ and wrong about what is _kept_. The latch releases when the
daemon comes up (`nextOnboardingLatch`), and anything still held came back on the way out.

Reproduced by driving the app rather than by reading it: open the Conflicts screen with a Details
dialog over it, wipe the daemon back to `firstRun`, let the takeover engage, then bring the daemon
back. **You finish first-run setup and land on the Conflicts screen with a Details dialog floating
over it** — both from before the wipe, both about a state that no longer exists.

Reachable rather than likely: it needs the daemon reset to first-run, or made unreachable _and_ its
folder pair removed, while the window is open with a layer showing. The cost of getting there is low
and the landing is wrong in a way the user cannot explain, which is the combination worth fixing.

Entering the takeover now discards both layers. **Edge-triggered on entry**, not asserted on every
render while the latch is set: the two are equivalent today, but the second would quietly forbid
onboarding from ever opening a layer of its own, and that is S7's call rather than this line's.

---

## The compact panel (F6)

Measured across the eight in-scope dark frames — `2a Compact settled/syncing/needs you`,
`4a Compact`, `10a Settled/Syncing/Offline/Paused` — and the three `12a` light twins.

## 58. No compact panel draws an attention band

`02-shell.md` §"The 360px compact panel" gives the panel four blocks, the third of which is an
**attention band**: `margin:12px 14px`, `border-radius:11px`, `padding:11px 13px`, a ring dot, 12.5px
/500 text and a `›`. Issue #170 repeats it. **Nothing draws it.**

The two frames that would carry one — `2a Compact needs you` and `4a Compact` — instead put a
full-width button where the band would go:

| frame                  | what the band's slot actually holds                                                   |
| ---------------------- | ------------------------------------------------------------------------------------- |
| `2a Compact needs you` | `316×38` button, `padding:10px`, radius `9`, 13px/500, `rgba(255,107,107,.1)` on `.4` |
| `4a Compact`           | two `332×61` deletion rows, then a `332×37` `Review them` at 12.5px/600               |

Frame wins (rule 2). The band is a 1040-screen component (`2a Needs you` draws one at 976×127) and
`bands.js` already has it; the panel says the same thing in a button, which is also the only thing
360px has room for. `ui/compact.js` therefore has no band, and the `›` glyph appears nowhere in it.

Worth stating plainly rather than leaving as an absence, because a reader of `02-shell.md` will look
for it and find a button.

### 58a. The panel's two deletion tints, and the light values nothing draws

§52a listed them and left them for whoever wrote the rows. F6 wrote them, as four tokens:

| token                                  | dark (measured)                  | light (derived)                |
| -------------------------------------- | -------------------------------- | ------------------------------ |
| `--compact-permanent-bg` / `-border`   | `rgba(255,59,59,.05)` / `.32`    | `rgba(220,38,38,.03)` / `.34`  |
| `--compact-recoverable-bg` / `-border` | `rgba(255,107,107,.035)` / `.26` | `rgba(190,18,60,.025)` / `.24` |

`4a Compact` has no `12a` twin, so the light column is derived the way §52 derived the attention
band's: by the delta each family shows between a card that IS drawn in both themes. Destructive moves
`.06`→`.04` and `.38`→`.40`; decision moves `.04`→`.03` and `.30`→`.28`. **S10 owns confirming them.**

The fifth token is not derived. `--compact-attention-border` — the panel's own edge when something
is waiting — is measured at `.3` in _both_ themes (`2a Compact needs you`, `4a Compact`,
`12a Compact needs light`). It is deliberately not `--decision-card-border`, which is `.3` dark and
`.28` light: the two coincide in one theme and part in the other, which is the exact failure mode
§52 records for the pair it had to split.

### 58b. The `12a` frames inherit a dark `color`, so the light twins are not mapped yet

The three light compacts were mapped, run against the gate, and taken back out. The panel needs no
new code in light: it reproduces `12a Compact settled/syncing/needs light` at every colour those
frames **declare**. What it cannot reproduce is the colour they **inherit**.

The prototype draws all sixty frames on one dark page, so every node in a `12a` frame that does not
set a colour of its own inherits `#F2F4F7` — the dark text tier — and the extractor records that as
ground truth. The app in light mode inherits `#14161A`, correctly, and fails on every one:
**142 failures across the three frames, all of them `color` or `border-*-color` reading
`rgb(242, 244, 247)` against `rgb(20, 22, 26)`, and not one of them a real difference.**

Fixing it means recording, per node, whether the prototype **set** a property or inherited it, and
treating an inherited colour on a light frame as a wildcard — which means regenerating all 51
fixtures. That is a change to the ground truth itself and it belongs with **S10**, which owns light
and needs the same answer for the seven screens that have no drawn light frame at all.

The measurement above is the useful part: it says the light mapping of this panel is already right,
and it names the one thing standing between S10 and proving it.

**CLOSED by S10 — see §91.** `extract.mjs` records `fromPage` per node and `assert.mjs` declines
those five properties on a light frame, counting and printing what it declined. The three compacts
are mapped and pass with **zero** failures, which is this section's prediction coming out exactly:
the panel needed no new code in light, only ground truth that knew what it did not know.

### 58c. Three defects F6 found in modules that were already merged

The compact panel was the first task to put a hexagon, a transfer row or an SVG colour in front of
the style gate — F4 mapped the shell's chrome and nothing else. All three of these would have failed
S1 through S10 identically:

- **`renderHexagon` emitted `flex:none` on every mark.** F2 wrote it on the strength of "a bare
  `<svg>` in a flex row shrinks, and every frame that sits in one declares it". Censused across the
  53 in-scope marks, **ten declare it and forty-three do not** — and `flex-shrink` is asserted. It is
  now the `flexNone` option, default off, and the ten sites are named in the source.
- **`.transfer-arrow` and `.transfer-detail` carried `flex:none`.** Neither declares `flex-shrink` in
  any frame that draws one (`2a Syncing`, `2a Compact syncing`, both light twins). Also inert:
  `.transfer-name` is `flex:1` on a `0%` basis, so neither ever enters the shrink calculation.
- **The gate compared SVG colour attributes as strings, which no themed mark can ever pass.** The
  prototype writes `stroke="#2E323A"`; the app writes `stroke="var(--hex-settled-track)"` and must —
  `tokens.css` is the only file allowed a raw colour, and light is a token swap. `assert.mjs` now
  compares `fill` and `stroke` as the **engine computes them** (`var()` resolves in a presentation
  attribute; both sides come out `rgb(46, 50, 58)`), which is what it already does for every style
  property. `url(#id)` matches any other `url(#id)`: the id must be unique per instance —
  `10a Glyph states` puts ten marks on one page — so the id is not design. `frames/*.json` is
  untouched.

  **The gradient that reference points at was not compared, for one commit.** `stop-color`, `offset`
  and `x1`/`y1`/`x2`/`y2` were in neither property list, so a syncing mark with its up and down
  gradients swapped — leaving files reading cool, arriving files warm — passed every gate. Closed by
  **#204**: all six are asserted now, and the swap fails with sixteen assertions. See §59.

### 58d. The tray panel's border is `#23262D`, not the translucent white the doc gives it

`10-tray.md` §"The panel" asks the tray form for `border:1px solid rgba(255,255,255,.1)`, with a
reason that is sound — it floats over the desktop rather than over the app surface, so an edge tuned
against `#0A0B0D` has nothing to sit against.

All four drawn tray panels use **`#23262D`**, the same `--border-chrome` every other compact panel
uses. Frame wins (rule 2), so `ui/compact.js` applies nothing extra for the tray form.

Recorded because S8 is the task that will feel the gap: it builds the borderless always-on-top window
this panel lives in, and if the edge really does disappear against a light wallpaper, the doc's value
is the intended answer and this is where the disagreement is written down. The `is-tray` class exists
already so there is a hook and no one has to reach for a `[data-state]` guess.

**Settled by §101** (#261, 2026-08-17): the doc's edge won. `is-tray` now styles it.

---

## 59. The gradient a syncing mark points at is now compared (#204)

The direction rule is one the design states outright — `01-foundations.md` §5, _warm = leaving this
computer, cool = arriving from Proton_ — and it was the one rule with nothing checking it. §58c
closed half the hole by comparing `fill`/`stroke` as computed values; the other half was that a
`url(#id)` reference is matched loosely, and the gradient it named carried no asserted property.

**Six properties close it**, split by which side each is written on:

| property            | prototype                                       | app                                 | so it goes in                                      |
| ------------------- | ----------------------------------------------- | ----------------------------------- | -------------------------------------------------- |
| `stop-color`        | `stop-color="#E55B2B"` (presentation attribute) | `style="stop-color:var(--up-from)"` | `STYLE_PROPS` — both compute to `rgb(229, 91, 43)` |
| `offset`            | attribute                                       | attribute                           | `SVG_ATTRS`                                        |
| `x1` `y1` `x2` `y2` | attribute                                       | attribute                           | `SVG_ATTRS`                                        |

`stop-color` being a _style_ property and not an attribute is the whole trick, and it is the same one
§58c used one layer up: the two sides write a colour in different syntaxes and an engine resolves
both. Listing it as an attribute would have failed on every stop — the app sets no such attribute.

`stop-opacity` is asserted too and records **nothing anywhere**: no stop in the design is
translucent, every element computes it to the initial `1`, so it costs the fixtures zero bytes and
closes the hole for the first stop that needs it.

**The regeneration is small and readable** — 252 lines across the 10 frames that draw a gradient, and
`stop-color` appears only on real stops because `INITIAL` carries its `rgb(0, 0, 0)` default (checked
against `div`, `svg`, `defs` and `linearGradient` before it was added). Assertions 11,199 → 12,441.

**Proved by breaking it**, the way F8 proved the style gate with a deliberately-wrong hex: swapping
`gradient(upId, "up")` for `gradient(upId, "down")` in `hexagon.js` fails 16 assertions across the
two syncing frames — `y1`/`y2` inverted on both gradients and all four stop colours warm↔cool — where
before the change it passed silently.

What is still not compared, and now genuinely is not: the gradient's `id`, deliberately, because the
app must make it unique per instance (`10a Glyph states` draws ten marks on one page).

### 59a. It also settles S10's gradient question, three tasks early

`12-light-theme.md` calls theme-aware SVG gradient stops _"the one structural edit light needs beyond
the mask colour"_, and offers two ways: duplicate the `defs` per theme, or drive the stops from CSS
variables. F2 took the second and verified it by sampling pixels. Extracting `stop-color` puts the
drawn answer in the fixtures, and the two sides match exactly:

| stop      | `2a Compact syncing` | `12a Compact syncing light` | token         |
| --------- | -------------------- | --------------------------- | ------------- |
| up 0%     | `rgb(229, 91, 43)`   | `rgb(178, 63, 20)`          | `--up-from`   |
| up 100%   | `rgb(255, 184, 77)`  | `rgb(217, 119, 6)`          | `--up-to`     |
| down 0%   | `rgb(6, 182, 212)`   | `rgb(14, 116, 144)`         | `--down-from` |
| down 100% | `rgb(59, 130, 246)`  | `rgb(29, 78, 216)`          | `--down-to`   |

The prototype duplicates its `defs` per theme; the app's single pair resolves to the same eight
values through `var()`. **S10 does not need to duplicate anything, and the day the `12a` frames can be
mapped (§58b) this becomes an assertion rather than a table.**

---

## Per-frame fixtures (F9)

---

## 60. Four more capability gaps, found by writing a dataset for every frame

The reconciliation sweep (P0.2) compared docs against frames and caught every place the two
disagreed. It could not catch this class, because these are not disagreements: the doc and the frame
say the same thing, and **nothing in the command surface can produce it.** An absence in a reply
shape is invisible until someone tries to fill the reply in, which is what F9 is.

Four, none of them among the ten capabilities in `IMPLEMENTATION-PLAN.md` §4 and none among G1–G4:

| #   | what is missing                                                                                                             | frames                                                                                                                 | issue                                                                   |
| --- | --------------------------------------------------------------------------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| G6  | a **size on a planned action** — `PlannedAction` has no size field at any level of the dry-run surface                      | `5a Plan` (`3 files, 4.1 MB`), `5a Plan safe` (a size per row), `9a Review` (`1.4 GB` / `38.4 GB`)                     | [#206](https://github.com/osirison/proton-drive-sync-engine/issues/206) |
| G7  | **index-wide totals** — how many files, how many bytes                                                                      | `2a Settled`, `2a Compact settled`, `10a Settled`, `7a Activity quiet`, `8a Settings`, `5a Checking`                   | [#207](https://github.com/osirison/proton-drive-sync-engine/issues/207) |
| G8  | a **subtree aggregate** for a directory about to be deleted, and an atime                                                   | `4a Deletions` (`1,204 photos, 8.4 GB`, `last opened Mar 2024`), `4a Armed` (the same count in the confirmation title) | [#208](https://github.com/osirison/proton-drive-sync-engine/issues/208) |
| G9  | ~~**dry-run progress**~~ — **CLOSED.** The rehearsal is a daemon-side pass now, so the existing `SyncActivity` describes it | `5a Checking` (`8,431 of 12,480 files`) — **drawn**                                                                    | [#209](https://github.com/osirison/proton-drive-sync-engine/issues/209) |

**G6 is not G2.** #191 is byte totals _per direction, per time window_ — what a pass moved. G6 is the
size of files a plan _would_ move, which is a field on a plan row. The distinction matters because
`9a Review` pairs its figure with `Needs 38.4 GB free. You have 214 GB.`, so it feeds C4's
free-space check: a **safety** statement resting on a number nothing computes.

**G8 is the other safety one.** `05-deletions.md` makes the count load-bearing — the armed
confirmation asks `Delete 1,204 photos from this computer?`, naming the magnitude in the question you
are answering. It is the one place in this design where a missing number is a safety problem rather
than a blank. The file card beside it (`4 KB`, `last edited Jan 2026`) _is_ answerable, via
`path_sync_status`, and the fixtures carry it — the two cards differ by whether a gap or a lookup
stands between the screen and its data.

**Phase-1 fallbacks are in the fixtures, at the point where the field would go.** No fixture invents a
shape for any of these, or for G1–G4: a plausible-looking `{ upBytes, downBytes }` would settle a
Phase-2 design from a preview dataset, which is the one thing the F9 contract forbids outright.

---

## 61. `See all 471 actions` is a derivation, not the plan's total

`9a Review` draws **both** `See all 471 actions` and `3 files can't be synced — a socket and two
shortcuts`, which looks like the frame contradicting itself. It does not.

`SkipUnsupported` is a `SyncAction` like any other, so a skipped file **is** a plan row and
`PlanSummary::from_plan` counts it: 128 uploads + 341 downloads + 2 conflicts + 3 skips is
`total: 474`. The button names the 471 that will actually happen — `total - skipped_unsupported`.

**S7 must render it that way.** Reading `summary.total` straight draws `474` and the frame says 471;
the three skipped rows are listed separately, immediately above, so counting them twice would be
visible on the same screen.

Recorded because the first version of the fixture resolved it the other way, by writing `total: 471`
next to an empty plan — a reply the daemon cannot emit, since `total` is `plan.len()`. The screen
built against that would have looked correct.

---

## 62. Where a light frame's dataset lives, and why it is a reference

All seven drawn light/dark pairs carry **identical text** — same node count, same tree keys, same
strings, walked in lockstep:

| light                       | dark                   | nodes | strings |
| --------------------------- | ---------------------- | ----- | ------- |
| `12a Settled light`         | `2a Settled`           | 26    | 12      |
| `12a Syncing light`         | `2a Syncing`           | 60    | 25      |
| `12a Conflict light`        | `3a Conflict`          | 75    | 42      |
| `12a Deletions light`       | `4a Deletions`         | 63    | 33      |
| `12a Compact settled light` | `2a Compact settled`   | 12    | 5       |
| `12a Compact syncing light` | `2a Compact syncing`   | 36    | 11      |
| `12a Compact needs light`   | `2a Compact needs you` | 13    | 6       |

Which is `12-light-theme.md`'s "Everything else is identical" measured rather than taken on trust,
and it settles how the fixtures are written: **there is no such thing as light-theme data.** Each
light entry names its twin (`sameAs`) instead of restating it, so the day S1 changes what
`4a Deletions` withholds, the light frame cannot keep the old queue.

The resolution inherits **data and never `fids`**, which keeps §58b's decision from being undone by
accident: the three light compacts were mapped, run and taken back out because the prototype draws
every frame on one dark page and a `12a` node that sets no colour of its own inherits `#F2F4F7`. If
`sameAs` inherited the mapping, mapping a dark frame in S1 would silently re-map its light twin and
reintroduce all 142 failures.

---

## The main screen (S1)

---

## 63. What the main screen cannot draw, and why none of it is a bug

The first screen to be built is also the first to meet the daemon's reply shape with a drawing in
hand. Four things `2a Syncing` and `2a Settled` draw have no data behind them, and the fallback in
every case is `14-behaviour-and-state.md`'s: **omit the clause, never fake it.**

| drawn                                                    | frame        | what exists                                                    | issue                                                                      |
| -------------------------------------------------------- | ------------ | -------------------------------------------------------------- | -------------------------------------------------------------------------- |
| `· 12,480 files · 41.2 GB` on the settled sub-line       | `2a Settled` | `last_sync_epoch_secs` and nothing else — no index-wide totals | G7 [#207](https://github.com/osirison/proton-drive-sync-engine/issues/207) |
| a 2px progress bar at the real percentage                | `2a Syncing` | see below — no percentage is computable at all                 | E1 [#98](https://github.com/osirison/proton-drive-sync-engine/issues/98)   |
| `386 MB sent · 1.1 GB received today` in the footer line | `2a Syncing` | nothing; the shell draws the folder pair instead               | G2 [#191](https://github.com/osirison/proton-drive-sync-engine/issues/191) |

**The transfer LIST is no longer one of them (#211, 2026-08-16).** `SyncActivity.transfers` carries a
bounded window — every row in flight, then the next planned transfers as `queued` rows, capped at
`TRANSFERS_REPORTED` (6, the design's own "~6 visible with `+n more`") — and
`transfers_remaining` is the untruncated count `+n more` is sized from. What did **not** become
drawable is a second row _in flight_: see §63c.

**The progress bar is unreachable by construction, not merely unimplemented**, and that is sharper
than #98 states it. `TransferActivity` carries `bytes_total` and `bytes_done` and **never both on the
same transfer**: an upload gets `bytes_total` from the local file's size and no `bytes_done` (the CLI
reports none), a download gets `bytes_done` sampled live from the staging directory and no
`bytes_total` (a remote listing carries no size). Neither direction can produce a fraction. So
`transferRow` takes `progress: null` meaning _no track_, distinct from `0`, which would read as
stalled — `rows.js` already carried that distinction for queued rows and it now has a second caller.

**The size chip is drawn on uploads and omitted on downloads** for the same reason, rather than
em-dashed: an em-dash means UNKNOWN in this design, and the daemon was never asked.

### 63b. `9a First sync`'s split bar disagrees with its own labels

The frame paints the sent fill **48px** and the received fill **88px** of a 400px track, and prints
`44 sent` and `115 received` directly underneath them. 48:88 is 0.55; 44:115 is 0.38. No denominator
produces both, so the two halves of the block contradict each other.

Its **total** is right: 48 + 88 = 136 of 400 is 34%, and the line above reads `159 of 471 done` =
33.8%. That is what identifies the split, rather than the extent, as the hand-drawn half.

So the app computes each fill from the count the same block prints — `uploaded_files / action_total`
and `downloaded_files / action_total`, 9.3% and 24.4%, summing to the 33.8% the frame's total already
agrees with. Reproducing 48:88 would mean drawing a split that contradicts the numbers beside it.
**Decided 2026-08-16**; two `decision` rows in `known-deviations.mjs` carry the exact widths.

### 63c. The second transfer row is _queued_, because the engine transfers one file at a time

`2a Syncing` draws an upload and a download **both in flight**, and `2a Needs you` and the compact
frames draw the same pair. The daemon cannot report that and it is not a gap in the payload:
`execute_plan_and_commit` is a sequential loop, so at any instant exactly one transfer is running —
or one _batch_, which is one `download_many` invocation over a chunk of files in one folder and is
reported as one row carrying `files`.

What #211 makes drawable is the rest of the picture: the queue. So the app draws the in-flight row
in its own column and the queue's next transfers in theirs, which on `2a Syncing` puts a `queued`
download where the frame has an active one. That row is a different construction — flat where the
frame's is wrapped, with no track — so it is left **unmapped** rather than allow-listed on fourteen
properties, the same call §79 makes for the remote folder card's `<input>`, and for the same reason:
comparing two differently-built nodes property by property is not a missing capability. The queued
row's construction is measured, on the left column of the same frame, which draws one too.

**Decided 2026-08-16.** A second _active_ row needs concurrent transfers in the executor, which is a
sync-engine change and not a reporting one.

### 63a. The style gate had no way to hold a recorded deviation

`2a Settled`'s sub-line renders `last synced 2 minutes ago` where the frame draws a 390px line, so
the node measures 195px and `box.w` fails — a **documented, planned** difference arriving as a red
build. Three ways out and only one is honest: filling the clause with plausible numbers is what §60
forbids outright; leaving the gate red makes it a gate nobody reads; so the assertion is recorded, by
frame, node and property, in `gui/tools/fidelity/known-deviations.mjs` with the issue that closes it.

The clause that keeps that from becoming a mute list: **an entry that no longer fails is itself a
failure.** The day #207 lands and the line grows its two clauses, the build fails until the row is
deleted. An allow-list that can only grow is a list of things nobody will remove.

---

## 64. The chip's count is a SUM, and the ring outranks the fill (§44 half-reopened)

§44 recorded two chip priorities as _chosen_ rather than measured, on the grounds that "nothing draws
decisions and deletions at once". `2a Needs you` draws exactly that, and the reason it did not look
like it is that the fixture said otherwise.

Read off the frame: the band's first row is `One file changed on both sides`, its second is
`Two deletions are waiting on you` / `1 removes from this computer permanently · 1 goes to Proton's
Trash`, the sub-line is `3 other changes are waiting on you`, and the chip is `3 waiting` with a
**1px ring** dot. So the frame's state is **one conflict and two deletions**, the chip's number is
their **sum**, and in their company the decision ring wins over the deletions fill.

The fixture had pinned three conflicts and an empty deletion queue — which makes every drawn _string_
correct and the band unrenderable, since no conflict can produce that second sentence. F9 wrote the
dataset from the frame's text and the text alone was consistent with both readings; it took building
the band to find out which.

**What survives from §44:** deletions still take the filled dot when they are alone, which is what
`4a Deletions` draws. What changes: the count is `conflicts + deletions`, and the ring is used
whenever a conflict is among them.

---

## 65. One transfer row, three tree shapes

`rows.js` builds one `transferRow`, and the prototype draws it three ways:

| site                            | row                                                     | its spans                              |
| ------------------------------- | ------------------------------------------------------- | -------------------------------------- |
| `2a Syncing`, in flight         | `display:block`, `position:relative`, `overflow:hidden` | inside a flex body, beside a 2px track |
| `2a Syncing`, queued            | `display:flex`, `gap:10`, `align-items:center`          | direct children, no track              |
| `2a Compact syncing`, in flight | `display:flex`, `gap:9`, `align-items:center`           | direct children, **with** a track      |

So the wrapper is not "the row that has a track" and not "the row on the main screen" — it is the
main screen's ACTIVE row and nothing else. Modelled as a `has-body` modifier that _resets_
`align-items` and `gap` to `normal`, because both are asserted properties and the frame records
neither on that row: inheriting the flex above would fail three assertions on a row that looks
identical on screen.

---

## 66. Two button colour roles the frames disagreed with, both invisible in dark

S1 mapped the first buttons the style gate has ever compared, and found `.btn` wrong at every site in
the app plus two kinds wrong in light only.

**`.btn` was `inline-flex`; all 175 drawn buttons are `block` (165) or `inline-block` (10).** The
split is just whether the parent is a flex container, which blockifies a `<button>`'s UA
`inline-block` — so saying nothing at all reproduces both groups, and the four properties F5
declared (`display`, `align-items`, `justify-content`, `gap`) each fail once per mapped button.
`line-height:1.2` goes with them: the frames leave it `normal`, and although the gate treats a
prototype-side `normal` as a wildcard, the box it produces is compared. F6 found this first and
fixed it inside `.compact-panel .btn` — right about the values, one scope too narrow. That override
is deleted and the finding lives in `.btn`.

**`secondaryFilled` reached for the surface tiers rather than the button role.** `--panel-raised` /
`--border` / `--text-2` measures correct on every dark frame and wrong on two of three in light:
`12a Syncing light` draws `Pause` at `#FFFFFF` / `#D6D2CB` / `#14161A`, where `--border` is `#E6E3DE`
and `--text-2` is `#374151`. Now `--btn-secondary-bg` / `--btn-secondary-border` /
`--btn-secondary-text`, which are identical in dark and correct in light. Nothing could have caught
it: S10 owns light and no light frame is mapped.

**`2a Settled`'s `Sync now` is a twelfth kind.** Four drawn instances — also `3a Conflicts cleared`'s
`Back to sync` and both of `9a Folders`' browse buttons — transparent over `#0A0B0D` in dark
(§1.3 conflict 4 already resolved that against the doc's `#101216`) and `#FFFFFF` over `#FAF8F5` in
light. No existing token is transparent in one theme and a surface in the other, so `secondaryOutlined`
gets `--btn-secondary-outline-bg`. Light is a lighter surface: a card that reads as depth there reads
as noise here, and the theme pair is the design saying so.

---

## 67. There is no 168px decision hero, and one state needs one

`14-behaviour-and-state.md` is explicit that needs-decision is additive — the hexagon carries the
transfer state and the band carries the decisions — and then: _"Only when nothing is transferring
does the hexagon itself take the decision form."_

§24 measured that the crimson mark **exists only at ≤72px**. Both are true and they only meet on a
screen nobody drew: idle, nothing in flight, three things waiting. S1 builds it from the rule — the
`needsNumeral` mark at 168 with the waiting count, `MAIN.compact.needYou(n)` as the headline (the
only sentence the deck has for this situation, quoted across surfaces the way `copy.js` exists for),
and the settled sub-line and buttons underneath.

**Prose-normative and unverified.** Same footing as the seam's 320ms transition (§38) and the ⋯ menu
(§45). Flagged for the designer with `paused` and `unreachable`, which are also specified in words
and drawn in no `2a` frame — `unreachable` borrows `TRAY.unreachableTitle`/`unreachableBody`, the
deck's own outage sentences, and `paused` renders `pausedSub` against `last_sync_epoch_secs` because
nothing records _when you paused_.

### 67a. The fourth undrawn state was a false all-clear, not a blank

`authExpired` had no branch at all, so it fell through to `settled` and drew **`Everything is up to
date`** on a daemon that cannot reach Proton — the one failure on this screen that is worse than
drawing nothing, because the screen's entire job is answering "is everything safe?" in under a
second. Found in review, not by any gate: the three drawn frames each exercise one branch, and no
frame draws this state.

It is not merely an undrawn state either. `routes.js`'s onboarding latch releases on `authExpired`
**specifically** so the main screen can carry it — _"onboarding can't fix expired auth… we must
actually hand off to the main screen's Re-authenticate action rather than trap the user in the
wizard"_ — so the fall-through broke a hand-off another module was written against.

Built from the one sentence the design has for it, `11-notifications.md`'s outage banner body:
`Proton Drive is asking you to sign in again. 61 changes are waiting — nothing is lost.` Split
headline/sub-line, which is the division of labour every other hero uses, and both halves are still
checked verbatim against `11a Outage`. The mark is the struck one it shares with `unreachable` —
that grouping is the design's own: _"an outage, expired session, or full disk"_ is one struck
`#FF3B3B` icon.

**`11a Outage`'s `Sign in` action is not built, and the button says `Try again now` instead.**
Nothing in the command surface signs in: the daemon reuses the `proton-drive` CLI's keyring session,
so re-authenticating is `proton-drive login` in a terminal. A `Sign in` button with no action behind
it is worse than the honest one, and `Try again now` is exactly right once the user has signed in
elsewhere. Related to E6 (#103), which would classify the state daemon-side; a GUI sign-in path is
its own product decision and does not exist.

### 67b. A seam site is a property of a transition, not of a mount

`updateMain` re-attached the seam node built at mount when the hero entered `syncing`. Entering it
from `decision` — a pass starting while something is already waiting — therefore attached the
`mainHero` seam, whose 150px overhang runs straight into the attention band: the rule-2 violation
`auditSeams` exists to report.

Recorded because of the class rather than the line. **A frame is one rendering, and this screen has
twenty transitions between six states.** Every gate here compares a rendering, so nothing in the
harness can see a wrong node re-attached on the way from one to another; the seam is rebuilt from
`seamSiteOf(next)` on every path that attaches it.

### 67c. §45a, one layer in: the band rebuilt itself on the poll

`fillBand` and `fillColumns` called `replaceChildren` on every status reply, so tabbing to `Compare`
and waiting dropped focus to `<body>` inside two ticks. Measured, not theorised — the same way §45a
measured it on the shell, with the same 1.2-second answer:

```
focused before: "Compare"
after 7s:       BODY
```

Which is the failure §45a exists to prevent, arriving in the first module written after it. The
shell's cache guards the shell's own nodes; a screen's list is the screen's problem. The band is
where it matters most: it holds `Compare` and `Review`, the two controls this screen exists to
offer, and `14-behaviour-and-state.md` requires every control to be keyboard-reachable "because this
is a desktop app".

Both lists now rebuild on a **signature** of their content — a row's file/direction/size/fraction, a
band item's title and note — and a poll that changes nothing changes nothing. Verified the same way:
focus survives on `Compare` across three polls.

**The general form, for S2–S10:** the shell patches rather than rebuilds, and every screen has to do
the same for its own lists. There is no gate for it — the harness reads a static tree, so a screen
that rebuilds itself twice a second passes every assertion in this repo.

### 67d. Two contradictory sentences in one window

`heroStateOf` split settled from syncing on `response.syncing` alone. A filesystem-watch event only
_accumulates_ `pending_changes` — `daemon.rs` says so in those words: _"filesystem-watch events only
accumulate `pending_changes` (they never trigger a reconcile)"_ — so for up to a scan interval after
an edit the daemon reports `syncing: false` with a non-empty queue.

`gui-core`'s `derive_state` already calls that `Running`, and the header chip reads off it. So the
chip said **`syncing`** while the hero underneath said **`Everything is up to date`**, about the same
file, at the same moment, six inches apart. Queued work is not settled; the hero now says so.

### 67e. Neither number in the syncing sub-line describes the pass

`started 14 seconds ago · 2 leaving, 1 arriving` needs the pass as a unit, and nothing in the reply
is one. Filed as **G11 ([#213](https://github.com/osirison/proton-drive-sync-engine/issues/213))**.

- ~~**`SyncActivity.since_epoch_secs` is the PHASE's start**~~ — **closed.** `begin_activity` reset
  it on every phase change, so a pass walking scanning → listing → executing → committing counted up
  and jumped back to zero three times, and `last_sync_epoch_secs` (the previous pass's _end_) was no
  fallback either. `SyncActivity.pass` now carries `started_epoch_secs` for the **pass**, held in
  `ControlShared` and stamped onto every activity rather than rebuilt with the phase; `subOf` reads
  it and keeps the phase's start only as the fallback for an older daemon.
- **`pending_changes` is the local watch queue and nothing else**, cleared after each successful
  reconcile. A pass driven entirely by Proton — a second device uploading, the first reconcile after
  a restart — has an empty queue while downloading, so the headline read `Syncing 0 changes` with a
  literal `0` inside the mark. Phase 1 takes `last_plan_summary.uploads + downloads` while syncing
  and falls back to the queue: both are the daemon's own numbers, deletions are excluded because
  "the count in the hexagon is transfers, not decisions", and on both drawn frames they agree at 3.
- **`last_plan_summary` is null until `execute_plan_and_commit` runs** — the whole scan-and-walk
  stretch, during which `syncing` is already true. `0 leaving, 0 arriving` is a summary the daemon
  never published, so the clause drops instead. Same rule as §63's omissions and `unreachableBody`'s.
  **Still the source of the headline count**, deliberately: `pass.changes` is populated at the same
  moment `last_plan_summary` is (both are set from the plan), so switching would move no number and
  only add a second path to one.

### 67f. `0 changes are waiting` at the moment the app can see nothing

`unreachable` is the one state reached with **no reply at all** (`derive_state` returns it only from
the `Err` arm), so the pending count is not low — it is unknown. Coercing it with `?? 0` printed
`Nothing is lost. 0 changes are waiting and will go as soon as it's back.` on a machine that might
have sixty-one queued.

`14-behaviour-and-state.md`, `gui-core`'s `DaemonState::Unreachable` doc and
`store.select.countersUnknown()` all forbid it in the same words: **unknown renders as an em-dash,
never as zero**. An em-dash mid-sentence is not English, so the clause goes and the reassurance
stays — the same shape as `MAIN.band.deletionSub` dropping a zero clause. Every `?? 0` on this
screen is gone; `count()` and the copy handle null.

---

## The cheap capabilities (C1–C5)

## 68. `deletion_policy` is two booleans, and two booleans have a fourth state

`8a Deletions tab` draws three radio cards under a mono key line reading
`deletion_policy · applies to both directions`. **There is no such key.** The three cards map exactly
onto `[delete_approval] remote` and `local`, which is why C1 (#174) is GUI work rather than engine
work: `remote` gates the recoverable direction (a file leaving this computer lands in Proton's Trash)
and `local` gates the permanent one (a file removed from disk is gone), so
`(true, true)` / `(false, true)` / `(false, false)` are the three cards, in the order drawn.

Three things follow, and all three are deviations rather than bugs.

**The key line names a key that does not exist.** Shipped as drawn — it is a label over two keys, and
G5 (#194) would mint the real alias. Note the same frame family draws `event_driven_reconcile` where
the engine's key is `events_driven`; `14-behaviour-and-state.md` is explicit about that one. Neither
mono key line is in `13-copy-deck.md` or `copy.js`, so **the copy gate cannot catch a wrong key
line** — S6 is on its own there.

**The fourth combination has no card.** `remote = true, local = false` — ask before sending a delete
to Proton's Trash, but wipe local files for good without asking — is reachable by hand-editing the
config and is a live safety policy. `DeletionPolicy::OnlyRecoverable` names it and
`is_drawn()` returns `false`; S6 must render **no card selected** rather than the nearest one.
Coercing it would mean the next save silently rewrote a setting nobody touched, in the one module
built not to do that. Writes always set **both** booleans, so choosing a card can never leave a
half-set table behind.

**The tab shows the file, not the running daemon.** `--no-delete-approval` forces both directions off
at runtime with nothing in the TOML, `.proton-sync.toml` in any directory overrides the daemon-wide
default per subtree (`src/dirconfig.rs`) with no UI surface at all, and there is no config-reload
path in the daemon — no SIGHUP handler, no watcher. So `Changes here take effect on the next sync`
(`08-settings.md`, the deck, and both 1040px frames) is not true as built: a save takes effect on
daemon **restart**, which is what `write_config`'s own comment says and what the existing restart
prompt does. S6 keeps the prompt and this row records the copy.

## 69. C2 walks the disk, because the index cannot answer the question the tab asks

`IMPLEMENTATION-PLAN.md` §4 and issue #175 both specify the same source — "`index_read.rs` already
reads the index; match each exclude glob against it" — and it cannot produce the drawn data.

The engine applies selective-sync filters to the local scan, the remote listing **and** the
base-index records _before_ planning, so an excluded file is never inserted. A rule that has been in
the config since before its files existed therefore matches **zero index records**. The frames prove
it on their own numbers: `8a Skip rules` names `exports/draft.tmp` and `exports/render-final.tmp` as
hidden by `*.tmp`, and `7a Never synced` says of those same two files _"They live in your folder but
no copy exists on Proton Drive."_ Never synced → never in `file_index`. An index-backed count answers
`0` for the row the frame draws populated, and fires `Matching nothing · safe to remove` over a rule
that is at that moment hiding a 40 GB export folder.

So `skip_rule_usage` walks the local tree — `read_dir` plus one `stat` per file, **no SHA-1**, which
is why it does not reuse `index::scan_local_files_with_options`. Consequences worth stating:

- **The walk is not the daemon's walk.** The daemon's scan prunes an excluded directory, so it never
  sees inside `video-raw/**`; this one must, or every directory rule would report zero. Traversal is
  decided by a _baseline_ `ScanOptions` carrying the config's includes but **none** of its excludes.
  The engine still owns every question about what counts (`should_ignore_path`, `.sync/`, the
  download scratch, glob semantics, non-UTF-8 keys) — none of it is reimplemented here.
- **A rule that prunes a directory owns everything under it.** A pattern can match a folder without
  matching any of its descendants by name (`node_modules`, `a/b`, `*/dir`), and the daemon stops at
  the folder. So each rule is asked `allows_relative_directory` on the way down and that answer is
  inherited, however deep. Classifying files alone reported zero for exactly those rules — see §72a.
- **An unreadable directory is skipped and counted, not fatal.** The engine's own scanner hard-fails
  there; one root-owned directory blanking the whole tab would be worse than a floor plus
  `unreadable_directories`/`unreadable_entries`.
- **Include globs ARE applied, and belong in the baseline.** A file outside the include list is
  already not syncing, so no skip rule is what hides it — crediting a rule with it would make
  `One rule removed — N files will start syncing` promise files that would not. The tab still lists
  only excludes; the includes are the Advanced tab's, and `skip_rule_usage` takes them so the two
  cannot describe different rulesets. (Omitting the argument does not fail, it silently widens every
  count — which is why the command's doc says so.)

### 69a. The fixture's byte discriminator does not survive real data

`8a Skip rules` draws two different sub-lines — `Skipping 2 files right now` (with paths) and
`Skipping 2 files, 3.1 GB` — and the F9 fixture distinguishes them by giving `*.tmp` `bytes: 0`,
which is what the frame's own arithmetic implies. **Live data has no such discriminator**: every rule
with files has bytes above zero, so the count-and-paths form would never render and the staged-removal
cost line for `*.tmp` would read `2 files, 0 B`. `08-settings.md`'s tab-2 table calls one second line
sample paths and another an added-date without saying when a row gets which. C2 returns every field
all three variants need — `files`, `bytes`, `unique_files`, `unique_bytes`, `samples`,
`folder_exists`, `error` — and **S6 owns the choice**. (A rule's `added 14 Jul` date has no source
anywhere: a TOML array of globs carries no per-entry timestamps.)

### 69b. Two "matches nothing" claims, one boolean

`Matching nothing` is about the count; `no such folder here any more — safe to remove` is about the
folder. They come apart — an empty folder that still exists matches nothing and is **not** safe to
remove — so the reply carries `folder_exists: Option<bool>` rather than a `stale` flag, and
`is_stale_folder()` requires the folder to be _known gone_. A rule with no literal folder prefix
(`*.psd`) makes no claim at all. The frame corroborates the split: `video-raw/**` matches files and
its second line still reads `the folder still exists on this computer`.

The removal-cost line reads `unique_files`/`unique_bytes` — files **only that rule** hides. A file a
second rule also hides does not start syncing when this one goes.

## 70. The conflict card's first sentence needs a version that no longer exists

`04-conflicts.md` gives the version cards three parts. C3 (#176) delivers the second — _what differs,
in words_ — and **cannot deliver the first.**

`You added a line, 5 minutes ago` is a claim about your version against the **last agreed** one, and
the last agreed version's content exists nowhere on the machine: the conflict sidecar is Proton's
copy _as it is now_, the local file is yours, and the index keeps the baseline's SHA-1 without its
bytes. Against Proton's copy alone the very same edit reads as a removal. This is not a harder
version of the diff problem; it is a different one, needing a capability nothing has. The relative
time in that sentence comes from the mtimes and is fine — the verb is the gap. Filed as **#217**.
`CONFLICTS.mineChange` / `theirsChange` stay in the deck as the drawn constants.

Related, and for S2 to resolve: `3a Conflict` draws `You added a line` while `3a Conflict diff` draws
the left side with four lines against the right's five, row 2 highlighted as a changed pair and row 5
absent. Under the alignment the frame itself draws, yours **changed** a line and added nothing — and
only that reading also makes `2 lines differ · 3 lines identical` true. The two cards were generated
by two different models.

### 70a. What the grammar covers, and what falls back to silence

`04-conflicts.md` is explicit that a summary which cannot be generated falls back to **the metadata
row alone**, and must _not_ fall back to showing the raw diff — that is what the disclosure is for.
`summariseSide` therefore returns `null` rather than stretching a sentence, for: more than one
changed line (quoting one of four describes the file wrongly, not partially), an extra line that is
not at the end (the only extra-line clause drawn is `an extra line at the end`), a blank changed line
(nothing to put in the mono span), and a difference no line shows. Two files with nothing in common
are refused too — 512 KB of text is ~10⁴ lines a side and an O(n·m) table over that would freeze the
window; the common-prefix/suffix trim makes the realistic case (one line changed in a large file)
cheap.

**Most real conflicts are multi-line, so most cards will show the metadata row only.** That is the
documented fallback rather than a hidden failure, and `comparison.changed`/`differing` are on the
reply for an S2 that would rather count than quote.

Two readings had to be settled from the frames rather than the prose:

- **`and is otherwise the same` is about ONE SIDE, not about the pair.** The left card claims it on a
  pair where Proton's has a line yours does not. Read symmetrically, the drawn sentence is unreachable
  at its own frame's input; read as _this side introduces nothing else_, both drawn sentences fall
  out of one grammar. Pinned by a test.
- **The two drawn forms are not one template.** The left contrasts the sides and closes with
  `otherwise the same`; the right, with an equally changed line, does not. Nothing explains the
  asymmetry, so the grammar reproduces both shapes rather than unifying them.

### 70b. Three things the pair reader cannot distinguish

`read_conflict_pair` returns `text: null` for a binary file, a too-large file, an unreadable one
**and** a missing one — and for the missing case `binary_or_large` is `false`, so a guard on that
flag alone reads a vanished file as an empty text file. `compare()` requires both texts to be
strings, which is the only guard that holds. A **directory** is indistinguishable from a JPEG
(`metadata()` succeeds, `read()` returns `EISDIR`, `size` is the inode size).

> **Amended by S2.** This used to close "so the type conflicts `04-conflicts.md` requires to hide
> the disclosure cannot be detected today", which was true of the pair READER and false of the
> engine. `read_conflict_pair` genuinely cannot tell a folder from a JPEG — but it is the wrong
> place to ask. `scan_conflicts` is already standing at the original with a path in hand, and one
> `symlink_metadata` settles it before any content is read. `Conflict.kind` now carries the answer
> (`content` / `type`), which is what makes the disclosure hideable and the queue's `a folder here,
a file there` renderable rather than hard-coded. The rest of this section stands.

Line endings are normalised before splitting. A sidecar written on another platform would otherwise
differ on **every** line, and `400 lines differ` is the worst possible wrong answer on a screen where
it argues for discarding a version. A difference that is only a trailing newline is reported as
`invisibleDifference` and suppresses every sentence: the files do differ — the daemon wrote a sidecar
— but no line shows it, and `Zero lines differ` would be a lie in the reassuring direction.

## 71. Free space: Phase 1 has the half the fallback assumes it has

`9a Review` draws `Needs 38.4 GB free. You have 214 GB.` and `14-behaviour-and-state.md`'s fallback
table says: _Free-space check on the local root | Onboarding step 2 | Omit the "You have 214 GB"
clause._ The fallback assumes **needs** is always known and only **have** can go missing.

**It is exactly inverted.** C4 (#177) supplies `You have 214 GB` exactly and always. `Needs 38.4 GB
free` cannot be computed at all: `PlannedAction` carries `path`, `destination_path`, `action`,
`entity_kind`, `conflict_path` and `remote_id` — **no size, at any level of the dry-run surface** —
so nothing can total the bytes a download plan would fetch. That is G6 (#206).

So neither the full sentence nor its documented fallback is renderable, and the residual
`You have 214 GB.` on its own is not a string in `13-copy-deck.md` or any frame. **S7 decides**
between omitting the line until G6 lands, adding a deck string for the have-only form, or shipping
the line only post-G6. C4 supplies the number and files the knot rather than settling it.

The command walks up to the nearest existing ancestor, because onboarding asks about free space
_before_ the folder is created — a plain `statvfs` on the chosen path returns `ENOENT` on the one
screen that needs the number. It reports `f_bavail`, not `f_bfree`: the difference is the
root-reserved pool, which a desktop app can see and can never write into, and this is a safety claim.

### 71a. #135, fixed at the funnel

Onboarding writes `local_root = "~/ProtonDrive"` — a literal the shell never touched. Every GUI
feature that joins that value onto the filesystem was operating on a directory named `~` under the
process's working directory: the conflict scan found nothing, the emblem lookup opened no index, and
the free-space check would have reported `ENOENT` on `9a Review`. The daemon has expanded `~` since
#134; the GUI did not.

Expanded once in `RuntimePaths::resolve` (before `db_path` derives from `local_root`, or the derived
index path inherits the unexpanded root), using **the engine's own** `expand_tilde` — now `pub`, and
wrapped as `gui_core::config_io::expand_config_path` so the Tauri crate keeps the facade. A second
implementation would be a second set of `~user` semantics to keep in step, which is the bug class
itself. A value the engine refuses comes back verbatim: that config is one the daemon will not start
on either, and the literal is what lets the error name the string the user typed.

## 72. The install command in `9a CLI missing` does not work on any distribution

The frame draws a command box containing `sudo apt install proton-drive`. This project's own
documentation contradicts it twice — _"`proton-drive` is not available in Linux distribution
repositories, so it can't be a package dependency"_ and _"The native packages deliberately do **not**
declare `proton-drive` as a dependency (it isn't in any distro repo)"_. There is no distribution
where a package manager installs it, so **for every user, including Debian users, the truthful
instruction today is the tarball/manual path.**

C5 (#178) ships detection, which is the half of this screen that works: `Detected Debian` tells
someone the app knows their system, and the help carries their instructions. The deck keeps the drawn
command verbatim, because the deck's job is to hold what the design draws and dropping it would
quietly remove the string from the copy gate — but `CLI_INSTALL_COMMANDS` carries the warning, and
**S7 must not ship a copyable command box from that table** until the design settles what the real
instruction is. Filed as **#218**; it needs a product call, not a code change.

**Settled by §102** (#218, 2026-08-17): the command box is dropped, not made conditional. `Detected
Debian` stays; `CLI_INSTALL_COMMANDS` and the copy gate row that checked it against this frame's text
are both gone.

Three smaller notes:

- **`/usr/lib/os-release` is read too.** Issue #178 names only `/etc/os-release`; the freedesktop
  spec's second location is what a stateless system ships, and following the issue literally reports
  "detection failed" there.
- **The name is a short brand name, not an os-release field.** The frame draws `Detected Debian`
  while `NAME` is `Debian GNU/Linux` and `PRETTY_NAME` is `Debian GNU/Linux 12 (bookworm)`. A
  recognised distribution gets its brand name from our own table; one known only through its
  `ID_LIKE` parent uses its own `NAME`, because `Detected Debian` on a machine that is not Debian
  would be worse than either the truth or silence.
- **There are two detection mechanisms in this repo.** `setup.sh` detects by _package-manager
  presence_ (`command -v dnf` / `apt-get` / `pacman`); C5 detects the _distribution_ via
  `/etc/os-release`, as the issue and the plan specify. They disagree on a machine with `apt`
  installed under a non-Debian id, and they are separately maintained tables. Not reconciled here —
  `setup.sh` is about build dependencies, this is about a user-facing instruction — but worth knowing
  they exist.

### 72a. Fourteen defects an adversarial review found, and where they lived

Three reviewers over the C1–C5 commit produced 21 findings; a refutation pass (every finding handed
to a second agent told to break it, with a real checkout to execute against) killed 7. **Zero of the
fourteen survivors were reachable by any gate**, for the same reason S1's nine weren't: the fidelity
and copy gates compare a rendering to a drawing, and none of this code renders anything yet.

They cluster, and the clusters are the lesson:

**Two matchers, one question.** `skip_rules` asked every rule a _file-shaped_ question, so a rule
that matches a **directory** and not its descendants — bare `node_modules`, `a/b`, `*/dir` — was
credited with zero while the daemon pruned the whole subtree. The tab would have drawn `Matching
nothing` and `One rule removed — 0 files will start syncing` immediately before deleting that rule
uploaded a `node_modules`. The module's own doc-comment describes exactly this failure and the test
that guards it uses `scratch/**`, whose `**` happens to match descendants — so the test passed and
the shape it was written for did not. Rules now carry `allows_relative_directory` down the recursion.

**Serialization is a boundary too.** `samples: Vec<PathBuf>` cannot serialize a non-UTF-8 path at
all, so one Latin-1-named file in one rule's first four matches failed the whole reply — every rule
on the tab losing its numbers over a filename none of them is about. Samples are display-only and
are now lossy `String`s.

**"No" has more than one meaning.** `skip_rule_usage` guarded "root not configured" and not "root
not _there_" — an unmounted external drive returned zero for every rule with `folder_exists: false`,
which is precisely `safe to remove`, on every rule at once. `distro::detect_here` conflated "no such
file" with "this file names something we have no command for", so a machine that overrode
`/etc/os-release` got the base package's `/usr/lib` answer contradicting it.

**A diff algorithm's shape is not its meaning.** `lcsOps` emits an edited block as _k_ removals then
_k_ insertions; pairing one op ahead matched the last removal with the first insertion and orphaned
the rest, turning two lines edited in place into one changed line plus a line each side had gained
— and slipping under the `changed.length > 1` refusal, so the card confidently described a file it
had misread. Related: a reordered file looked like a gain (LCS scores a move as remove + insert), an
extra line before a changed one was called "at the end", `differing + identical` could exceed the
file's own length, a whitespace-only difference rendered two identical quotes each insisting the
other was different, and `slice()` cut surrogate pairs in half. Each is now a refusal or a fix with a
named regression test.

**Null is a shape, not an error.** `versionDiff(side, null)` threw — and `summariseSide` returns null
for the _most common_ real conflict, a multi-line edit. `diffSummary(0)` rendered `zero lines
differ.` Both now answer `null`, so a caller's `if (sentence)` is the documented fallback.

Three of the seven refutations were of the form _"unreachable — nothing calls it yet"_. That is
true and is not a reason to leave it: the caller is S2 and S6, and this is the commit that decides
what they find. They were fixed anyway.

Two Copilot findings landed in the same pass and were real (`check_cli` requiring exit-zero while
its own doc said _presence_; an off-by-`n+m` cell cap). A third — that `Option::as_slice` does not
compile — was answered rather than applied.

### 72b. C1 is wired, not just written

The review's sharpest process finding: `DeletionPolicy` had no caller outside its own tests.
`ConfigPayload` returned the two raw booleans and `ConfigUpdate` accepted them, so the four-state
guarantee — including the undrawn `OnlyRecoverable` and the `absent means true` defaulting — was
unenforceable from the UI, and S6 would have re-derived it in JavaScript. Both structs now carry
`deletion_policy` alongside the raw pair: the pair is what the config text shows, the policy is what
a radio group binds to, and the defaulting lives in one place.

## 73. C6 is not in this PR

`IMPLEMENTATION-PLAN.md`'s suggested order reads `S1 → C1–C5 → S2 …`, omitting C6, and the omission
is right: #179 is `notify_policy` **plus the four notification triggers**, 30-second coalescing and
never stacking two banners. That is S9 runtime behaviour, not a capability shim, and it is built with
the screen that owns it (#188).

---

## Phase-1 capability deviations

`IMPLEMENTATION-PLAN.md` §4 lists the ten daemon capabilities the design assumes and which four are
Phase 2. Each Phase-1 fallback gets a row here as its screen lands — G1–G5 close them.

| screen | frame            | drawn                                  | Phase 1 draws                       | closed by |
| ------ | ---------------- | -------------------------------------- | ----------------------------------- | --------- |
| S1     | `2a Settled`     | `· 12,480 files · 41.2 GB`             | the timestamp alone                 | G7 #207   |
| S1     | `2a Syncing`     | a second transfer **in flight**        | the queue's next row there (§63c)   | (decided) |
| S1     | `2a Syncing`     | a progress bar at the real percentage  | no track at all                     | E1 #98    |
| S1     | `2a Syncing`     | `386 MB sent · 1.1 GB received today`  | the folder pair (the shell's line)  | G2 #191   |
| S7     | `9a Review`      | `Needs 38.4 GB free. You have 214 GB.` | the free-space half only (§71)      | G6 #206   |
| S2     | `3a Conflict`    | `You added a line, 5 minutes ago`      | the relative time (§70)             | G12 #217  |
| S7     | `9a CLI missing` | `sudo apt install proton-drive`        | the manual path for everyone (§102) | (decided) |

### 74. S2 · the conflicts screen

**A choice button paints nothing itself.** All three buttons on `3a Conflict` put every string in a
span with its own colour, so the element keeps the UA's own button colour — measured `rgb(0, 0, 0)`
on `3a Conflict` and again on `12a Conflict light`, theme-invariant because it is never seen. It is
`--btn-unpainted` rather than the literal it looks like: `check-tokens.mjs` allows no raw colour
outside `tokens.css`, and `revert` would not reach it either, because the app declares
`color-scheme: dark` and the prototype declares none, so the UA's `buttontext` is a different black.

That also made F5's `decisionChoice` wrong in a way no gate could have caught: it carried
`color: var(--text)` at weight 400, read off the frame's _label_, which is `#FF9C9C` at 600 on a
span. Nothing rendered the kind until S2 — the same undrawn-code class as §63's nine and C1–C5's
fourteen, and the third time review rather than a gate was what found it.

**`Keep both`'s two themes disagree about whether a border exists.** Dark draws 1px
`--border-strong`; light draws none. Not a rounding artefact — the inner label row measures 281.33px
dark against 283.33px light, exactly the 2px a border takes out of a border-box. Hence
`--btn-primary-choice-border-style`, the second token after `--btn-secondary-outline-bg` whose two
themes disagree about whether a surface is there at all.

**Both diff tints are CHOSEN in light.** `12a Conflict light` mirrors the CARD view, so no frame
draws the diff panel in light. Derived by §54's method: the nearest same-family drawn pair is the
warn band (`--warn` `#FF9F1C` darkening to `#B23F14`, `.04 → .03`, ×0.75) and the background column
as a whole centres on ×0.7, so the drawn `.14` maps to `.105` and `.098` — which agree at `.10`.
S10 confirms. The panel's three greys are role twins of `--seam` / `--text-disabled` /
`--border-strong` and take their light values from them.

**The crossfade is a fade-in.** `04-conflicts.md` asks for a 220ms crossfade on advancing between
conflicts. A true one needs both bodies alive and stacked, which needs a positioned wrapper — and
`renderConflicts` returns window-root SIBLINGS precisely so the seam's `left: 50%` resolves against
the 1040px window. So the new body fades in over 220ms and the old one is simply gone. Applied only
on an advance, never on the first mount and never on a poll, so the gate reads
`animation-name: none` the way the frames do.

**A quoted line loses its list marker.** `3a Conflict` quotes `buy milk` out of a line that reads
`- buy milk`. Stripped for `-`, `*` and `+`, and deliberately not for `1.`: a numbered item's number
is content the reader may be pointing at, and dropping it could quote two different lines
identically.

**`Open both in an editor` is drawn and inert.** No command opens a path, and `commands.rs` is
explicit that a screen task never adds one — a screen that needs something the surface lacks files a
C-item, which adds the command _before_ the screen is built. S2 found the gap while wiring the
screen, too late for that. #220.

**`CONFLICTS.kindBinary` is written, not measured.** No frame opens a conflict whose pair has no
readable text, and the deck carries no wording for one. `kindFolder` sits beside it for the same
reason and survived a near-miss worth recording: `3a Conflict diff`'s queue list draws
`photos/trip · a folder here, a file there`, which looks like the drawn instance the exemption
denies. It is not — that string is `typeConflict`, the LIST wording, while `kindFolder` is the meta
line under a filename on a card, and no frame opens a type conflict card. Rewriting one to match the
other made the copy gate green by pointing two deck entries at one drawn node, which is the exact
failure a verbatim gate exists to prevent.

**The cleared state is a 522px window and the shell has one fixed size.** 15 assertions, #221 — the
issue carries the table. The body renders as a centred 520px column, which is the closest the shell
can get; the footer is a child of the window rather than of the body, so it stays 1040 wide.

**Settled by §103** (#221, 2026-08-17): the shell does not resize. Rows recorded, not deleted.

**A fixture may now name its own route.** Until S2 nothing needed to: every mapped frame was either
the main screen (the default) or a compact panel (intercepted earlier). So `?frame=3a Conflict` drew
the MAIN screen against the conflicts fixture — worse than a blank screen, because `fid()` keys by
slot NAME, `main.js` stamps `fid(view.mark, "hexagon")`, and the conflicts table also has a
`hexagon`. The gate compared the 168px main hero against a 44px on-seam mark and called it a size
failure on a screen nobody had rendered.

**The app mark's wrapper repainted the mark.** F4 wraps the bare `<img>` in a `<button>` so it can
be a home affordance, and three UA button defaults then landed on the image: `font-size: 13.3333px`,
`text-align: center`, and a `color` of `buttontext` — which under `color-scheme: dark` is WHITE
where the frame inherits `#F2F4F7`. None is visible on an image and all three are compared. Present
on every overlay screen since F4; S2 is simply the first mapped one.

| screen | frame                  | drawn                             | Phase 1                         | gap      |
| ------ | ---------------------- | --------------------------------- | ------------------------------- | -------- |
| S2     | `3a Conflict`          | `· last agreed 3 hours ago`       | the file kind alone             | G12 #217 |
| S2     | `3a Conflict diff`     | `Open both in an editor` opens it | a button that does nothing      | #220     |
| S2     | `3a Conflicts cleared` | a 522px window                    | a 520px column in a 1040 window | decision |

### 75. S3 · the deletions screen

**`Keep it` has a command now, and it is one primitive for both directions (#224, landed).**
`ControlCommand::Deny` only ever revoked an approval — withholding is already the default, so denying
something nobody approved was a no-op, and neither half of the button's own sentence happened: the
refusal was not durable (the planner re-derived the same withheld action next pass, so the row came
back at the next launch) and the other side was never restored. `ControlCommand::Keep` purges the
deletion's baseline `file_index` record — its whole **subtree**, because a directory deletion is
planned recursively with every descendant suppressed — so the surviving side stops looking like a
delete and starts looking like a fresh copy, which the bootstrap arm adopts back onto the other side.
The queue empties because the planner stops deriving the action, not because the screen remembers an
answer. `deletionsDecided` survives as a two-second window over the poll rather than a session-long
fiction.

It also latches a **full walk**, and that is the half a unit test against a full-snapshot fake would
have missed: an incremental pass builds its remote map as `reconstruct_remote(base ⊕ delta)`, so the
baseline IS the remote view. Purge the record and a `direction: remote` survivor is in no
reconstructed map, and no event re-derives it — the deletion happened _here_, so the volume stream
never mentioned it. `bring it back to this computer` would have downloaded nothing, ever, with the
periodic resync off by default and every restart warm-starting from the cursor.

**The screen may not be rebuilt on the poll, and S2's may.** `4a Deletions` puts a typed-`DELETE`
field on every permanent card, and `14-behaviour-and-state.md` requires it to clear on blur — so the
~2s rebuild that the conflicts screen is free to do would wipe a half-typed word twice a second and
leave the app's only irreversible action unreachable by keyboard. `updateDeletions` rebuilds only when
a signature over the queue changes and otherwise refreshes the relative times in place. The signature
deliberately excludes those times, which is why they need refreshing at all.

**Esc is taken before `closeOverlay`.** The armed confirmation is a BODY of the deletions screen
rather than a route — `4a Armed` keeps the header and the four doors, and a confirmation about one
specific item has no id you could navigate to without naming the item. F4 had provisionally listed an
`armed` route; leaving it in was worse than unused, because `navigate("armed")` would have drawn the
"not built yet" placeholder for a screen that is built. It is gone, and `Press Esc to cancel.` works
because app.js takes the key ahead of the overlay stack. Left to `closeOverlay`, Esc dismissed the
whole queue — the frame's own caption doing the opposite of what it says.

**Two numbers landed and one word cannot (#208).** `PendingDeletion` now carries `subtree_files` and
`subtree_bytes`, counted from the already-filtered baseline at gate time, so the folder card's
consequence wraps to its drawn two lines, the armed title states its own magnitude and `armedBody`
gets its `— 8.4 GB —` clause back. What no engine can produce is the **noun**: the frame says
`1,204 photos` because its folder is a photo library, and a UI guessing that from the extensions
inside would be inventing a fact on the one screen whose job is not to. The app says `1,204 files`,
which is 17.71px narrower on the card and 37.07px on the armed title — three `decision`-class rows in
`known-deviations.mjs`, dated 2026-08-15, and not a re-drawn frame's worth of difference. `last
opened Mar 2024` is still an atime the index does not store at all.

The armed title's row is new, and it is the coincidence below arriving: Phase 1's
`Delete photos/2019 from this computer?` measured 507.56 against the drawn 507.73 and passed inside
the tolerance. `Delete 1,204 files from this computer?` does not, so the difference that was always
there is now visible to the gate.

The shapes below differ on purpose, and two of the four are how the sentence reads when the daemon
cannot count (an older daemon, or an empty folder — `Deleting this removes 0 files` is a sentence
about nothing):

- The consequence becomes a _different sentence_ (`Deleting this folder removes everything inside it
from this computer.`) rather than the drawn one with the figure omitted, which would read "Deleting
  this removes from this computer". It keeps the emphasis on the loss — `everything inside it`, true
  and qualitative — so the card keeps its crimson span and its structure.
- `armedBody` takes a **null size** and drops the whole `— 8.4 GB —` clause, the shape
  `MAIN.syncingSub` uses for a null plan summary. An em-dash between two em-dashes would be the app
  claiming the daemon answered "unknown" about how much is at stake.
- `armedTitle` takes the noun phrase instead of a count, so one grammar serves both: the frame draws
  it at `1,204 photos` and Phase 1 passes the path. **The two happen to measure the same.**
  `Delete photos/2019 from this computer?` is 507.56px against the drawn 507.73 — inside the 0.5px
  tolerance, so the gate is green on a node whose TEXT differs from the frame. Recorded here because
  no row can record it: a deviation entry has to fail to exist.
- `last opened` is omitted entirely, one drawn node the app does not render. `assert.mjs` walks the
  app's stamped nodes, so an absent node is silently unasserted rather than failed — worth stating,
  since it is the one Phase-1 omission the gate cannot count.

**Eight deletions in one column is 2,365px of content in a 764px window that cannot scroll.**
`4a Deletions` draws one card per column and fills 528px exactly, so the frame cannot show it: past
one card the last seven and the four doors are simply unreachable. `02-shell.md` calls this "a real
bug found twice during this design" and F4 already wrote the discipline into `.content-region` — a
list that can genuinely exceed its space opts into scrolling. `.dl-columns` takes it PER RENDER
rather than always, when any column holds more than one card, because one card per column is the
drawn arrangement and it does fit: a screen showing what the frame shows should behave as the frame
does, scrollbar included. The rule is "more than the drawn arrangement" rather than a measurement,
which is honest about what it is — one card whose consequence wrapped to five lines would still
overflow, and no queue produces that. Found by rendering the screen against queues no frame draws;
that method is worth a gate of its own, and it is not one today.

**`deleted on Proton 22m ago` was the age of the PASS, and is the deletion's own now (#225, landed).**
`detected_epoch_secs` reads like when the deletion happened; `decide_delete_gate` stamps `now` on
every withheld action, `self.pending_deletions` is replaced wholesale at the end of every plan, and
the incremental fast-path explicitly cannot idle-skip while anything is pending — so a deletion that
happened three days ago reported an age of seconds, refreshed every ~30s. That was a live bug, not a
missing capability: the field was already on the wire and already wrong. `first_seen_epoch_secs` is
the fact, carried across passes and restarts in the daemon's own `withheld_deletions` table and
re-stamped only when the fingerprint changes (a different deletion at the same path). `0` on the wire
means _an older daemon cannot say_, and the clause is omitted rather than aged from 1970.

It gives the folder card a strip again — one fact, since its other is the atime — and puts the file
card's two facts in the drawn order. `factsOf` still returns each fact with its DRAWN slot, because
the second is what a folder omits now rather than the first.

That also moved the card's one fact from the frame's `span[0]` to its `span[1]`, which the mapping
has to state rather than derive. `factsOf` returns each fact with the DRAWN slot it stands for;
stamped by DOM position, the app's `last edited` was compared against `deleted here 6m ago` and
reported as a width failure on a correct card. The conflict card gets away with position because it
only ever omits from the end.

**The armed confirmation had folder grammar for a file.** `Everything in archive/old-notes.md is
removed from disk.` — both drawn permanent items are folders, so no frame and no gate covers the
case, and `DeleteDirection::Local` on a file is the common shape rather than an edge. The deck gained
`armedBodyFile`, sharing a tail with `armedBody` so the promise about what cannot be undone is the
same sentence either way.

**The path's slot in that sentence is found by the template, not by searching for it.** `indexOf`
finds the first textual match, and the sentence has words before the hole: a folder named `in`
matched inside `Everything` and the mono span wrapped two letters of the first word. Rendering the
same template around a marker asks the deck where its own hole is.

**The empty state is a 522px window, exactly as the conflicts cleared state is.** Three assertions,
#221. `4a Empty` draws neither the header nor the doors, so its fixture declares no shell slots at
all; the body renders as a centred 520px column inside the 1040 window and is `flex: 1` tall rather
than 420.

**Settled by §103** (#221, 2026-08-17): the same decision — the shell does not resize — settles this
frame too, on the precedent the decision itself names. Rows recorded, not deleted.

**`primarySoft` was three wrong tokens in light — §66, one screen later.** F5 built `Keep it` from
`--panel-raised` / `--border-strong` / `--text-bright`, exact on every dark frame and wrong on all
three values in light: `12a Deletions light` draws `#14161A` / no border / `#FAF8F5` where those
tokens are `#FFFFFF` / `#E0DCD5` / `#14161A`. In light this button simply _is_ the primary button,
which is `12-light-theme.md`'s own rule ("`Keep both` and `Keep it` stay the loudest thing on their
screens") reading as a token identity rather than a resemblance. Hence `--btn-primary-soft-*`, whose
border is the second after `--btn-primary-choice-border` whose themes disagree about whether a border
exists at all: 38px dark, 36px light, the 2px a border takes out of a border-box.

**Three F5 controls were wrong the moment something rendered them**, and none of the three could have
been caught before S3 — the fourth, fifth and sixth instances of the undrawn-code class §63 opened.

- `.delete-gate` was `text-align: center; max-width: 160px`. Both drawn gates are left-aligned and
  neither is 160 wide (`4a Deletions` flexes to 330.64, `5a Plan` is 190), and both draw `#F2F4F7`
  where `.input` gives `--text-2`. The width belongs to the site, so there is none in the control now.
- `destructiveDisabled` borrowed `--destructive-border` (.38) where the frame draws
  `--destructive-btn-border` (.25) — the same hue one step brighter, and F1 had tokenised the pair
  with this exact site named in its comment.
- `.deletion-gate-row .btn { flex: none; padding: 10px 17px }` was dead in one half and wrong in the
  other: controls.js writes padding inline, which no rule can reach past, and the frame leaves the
  button at the flex default. The geometry is passed at the call site now, as `keepButton`'s own
  comment already said it had to be.

**A borderless button kind was drawing a 1px transparent border.** `--btn-border` fell back to
`transparent` with the style left `solid`, so `Delete permanently` measured 172.27×42 against the
drawn 170.27×40 and reported `transparent` where the frame records `rgb(255, 255, 255)` —
`currentColor`, which is what a browser reports for a border nobody draws. Both halves are fixed in
`applyKind`: no border means style `none` and colour `currentColor`.

**The status chip stopped being asserted the moment it changed variant.** `renderHeader` stamped the
chip's `data-fid`; `updateHeader` REPLACES the chip node when the variant changes, and the
replacement arrived unstamped. Every frame whose chip is not `idle` reaches its variant that way —
the first render happens before the first status reply — so the chip and its dot have been
unasserted on every mapped frame that has one since S1, S2 included. Silently, because `assert.mjs`
walks the app's stamped nodes: a node that stops being stamped stops being compared. `statusChip`
stamps itself now, so both paths get it. Measured by reverting the stamp and re-running the gate:
**696 assertions**, across the twelve mapped frames whose chip is not `idle`. (This screen's total
went 28,018 → 33,275; the other 4,561 are its own three bodies, which the 28,018 baseline was not
comparing either — the first render threw inside `renderHexagon` and killed every render after it,
so the three `4a` frames were reporting header failures with unstamped bodies behind them.)

**A literal NUL made the whole screen a binary blob, and every gate stayed green.** `itemKey` joined
its two parts with U+0000 written as the byte rather than as the escape. Prettier formatted the file,
eslint linted it, the tests passed and the harness rendered it — while git classified the blob as
binary, so `git diff` printed `Binary files differ` for 600 lines, `git blame` and `git grep`
returned nothing, and a review agent handed the diff could not read the code at all. The tell was a
`grep` over the file silently finding no matches, which reads as "the code is not there" rather than
as a broken file. `gui/tools/check-sources.mjs` now fails the build on any invisible character in any
source file, and it caught itself on the first run: the pattern and the name table spelled the
characters out, nineteen times, which is the whole argument for the rule.

**The busy set left the render signature, and a rebuild no longer eats a half-typed word.** Folded
into the signature, clicking `Move to Proton's Trash` on one card rebuilt the body and emptied the
gate on the other — the exact failure this screen's patch-not-rebuild path exists to prevent, one
layer in. Each control now registers an `apply(busy)` that sets what a rebuild would have set. A
rebuild is still sometimes right (a `path_sync_status` reply genuinely changes what a card draws), so
the typed word and its caret are carried across one, keyed by item: the field clears on BLUR by
design, and a rebuild is not a blur.

**A no-op reply was being read as a decision.** `apply_approval_command` answers `Ok("no pending
deletion matches '<path>'")` when the selector is absent from the snapshot it holds — which the GUI
can reach by acting on a queue up to two seconds stale — and the envelope check treated it as
settled, hiding a row nothing was recorded for. The check is now a positive match on the daemon's own
`approved N …` / `denied N …` acknowledgement, so the failure direction is safe: reword the daemon
and the GUI stops recording decisions rather than hiding deletions that never happened.

**`severityOf` fails CLOSED, and the first version failed open.** Written as `=== "local" ?
permanent : recoverable`, anything the wire sends that is not exactly `local` lands in the
recoverable column — which has no typed gate and whose one button approves in a single click. A
missing field, a typo, or a third `DeleteDirection` added upstream would have turned a permanent
removal from this computer into a one-click action, which is the precise failure this screen exists
to prevent. It asks for `remote` instead, so an unrecognised direction gets the gate. It does not
throw, where `transferSlotOrder` in the same module does: an unknown transfer direction is a bug in
the app and the throw is how it gets fixed, while an unknown delete direction arrives off the wire
mid-render, on the screen you least want to blank.

**`severityOf` moved to `ui/rows.js`, because more than one surface asks it.** The attention band was
counting `d.direction === "local"` inline — a second derivation of the rule the Deletions screen
sorts its two columns by, agreeing with it only by hand.

**Two harness gaps, both found by being the first to need them.** `check-fixtures.mjs` probed factory
slots with two arguments and S3's fact strip is keyed by three (column, card, fact), so a correct
mapping interpolated `span[undefined]` and failed the build. And `compareSvgAttr` could read only hex
colours, while `4a Armed`'s warning hexagon fills with `rgba(255,59,59,.08)` — the same colour the app
resolves `rgba(var(--destructive-rgb), 0.08)` to, matching on nothing.

**Six drawn strings were in the deck and not in the module.** `13-copy-deck.md`'s Deletions section
lists the facts strip in full — `deleted on Proton 22m ago` · `last opened Mar 2024` · `deleted here
6m ago` · `last edited Jan 2026` — and F7 left all four out of `copy.js`, along with the count in
`title` and the path in `armedBody`. The copy gate compares the MODULE against the frames, so a
sentence missing from the module is invisible to it; the gate can only ever catch a string that
drifts, never one that was never written down. All six are templates in the deck now and all six are
in the gate's `DRAWN` table.

**`.deletion-kind` keeps `--text-4`, and the light frame disagrees.** `12a Deletions light` draws
`a folder` / `4 KB` at `#6B7280`, where the same node is `#828B98` in dark and every other `#828B98`
node in the frame moves to `#4B5563`. `12-light-theme.md` maps the quiet tier two-to-two
(`#828B98 → #4B5563`, `#6D7783 → #6B7280`) and says so again in prose: _"Metadata inside those cards
moves from `#828B98` to `#4B5563` — on a light tint the quiet tier is too quiet."_ The doc is
normative for tokens under §1.3 rule 1, so the token stays and the frame's value is the slip. No gate
sees either way — no light window is mapped (§58b) — which is exactly why it is written down before
S10 propagates the table.

**`DELETIONS.compact.permanent` still hardcodes the aggregate, and nothing live reads it.**
`1,204 photos gone from this computer, permanently` is a fixed string in the deck and drawn in
`4a Compact`, so the copy gate is happy and the panel reproduces the frame. It is the same number the
card now states for real (#208) — but `trayView`'s `PANEL_STATE` has no `deletions` form, so the live
panel never builds that body and the string is reachable only from the fixture. The day the tray
panel renders a live queue it needs what the card and the banner got here: `subtree_files`, and the
same null-propagation rule when one row cannot be counted.

| screen | frame          | drawn                                     | shipped                                  | gap      |
| ------ | -------------- | ----------------------------------------- | ---------------------------------------- | -------- |
| S3     | `4a Deletions` | `1,204 photos, 8.4 GB`                    | `1,204 files, 8.4 GB`                    | decision |
| S3     | `4a Deletions` | `last opened Mar 2024`                    | the clause omitted                       | G8 #208  |
| S3     | `4a Armed`     | `Delete 1,204 photos from this computer?` | `Delete 1,204 files from this computer?` | decision |
| S3     | `4a Empty`     | a 522px window                            | a 520px column in a 1040 window          | decision |

### 76. S4 · the plan screen

**There are four bodies, and the frames draw three.** `14-behaviour-and-state.md`'s empty-and-error
table specifies the fourth in prose — _"dry run failed → show the daemon string, offer `Check
again`"_ — and it is the state a machine with no `proton-syncd` on its PATH reaches on the first
click. Without it the screen renders nothing at all. Its two sentences (`Couldn't work out what would
change` / `Nothing has been touched. This is what it said:`) are S4's rather than the deck's and are
exempted in `copy-gate.mjs` with that reason; the daemon's own message is quoted exactly, in mono,
and passes through no formatter (voice rule 4). The block is composed from drawn parts: `4a Empty`'s
centred 520 column, `3a Conflicts cleared`'s 88px mark, and `6a Activity passes`' quoted-error box.

**This is the one screen whose FOOTER changes with its own state.** `routes.js` records a footer per
route from a 13-to-6 census, and the plan route is not one answer: `5a Plan` and `5a Plan safe` draw
a footer action bar, `5a Checking` draws the four doors. So the shell asks the screen
(`footerKindOf`) instead of the table. The failed body takes the bar, because `Check again` has to
live somewhere and the doors are not somewhere.

**`5a Checking` draws no status chip, and the app draws one.** The frame's header is `mark · name ·
spacer · ⋯` — three slots where the other two `5a` frames have four. `06-plan.md`'s Behaviour section
says the opposite in prose: the chip reads `rehearsal · nothing has changed` **the whole time**, and
the checking state is the middle of that time. The doc is normative for semantics under §1.3 rule 1
and the frame for per-element geometry, so the app keeps the chip and the slot is simply not declared
in `planFids` — nothing is compared against a node the frame does not have. The chip also OUTRANKS a
waiting decision while this screen is open, which is a real precedence choice: it is the screen's
promise that nothing you are looking at has happened, and `3 waiting` in its place would answer a
different question in the one corner of the window that could reassure you.

**The safe screen is chosen on "every action is a file crossing the seam", not on "nothing is
destructive".** `5a Plan safe` is two lists of files and has nowhere to put anything else, so a plan
holding a conflict, an adoption, a type clash or a purge would render as a complete, calm screen with
an action silently missing — on the screen whose entire promise is that it lists every one. Those
plans get the list body; the destructive band appears within it only when something is actually
gated. Both drawn frames come out exactly as drawn. An action this app has never heard of is treated
the same way, which is why `sideOf` answers `null` by default rather than guessing a side.

**The two destructive sets are not one set, and `plan.rs` already said so.** _Display-destructive_
(tinted, sorted first) is `remote_delete | local_delete | purge`; _gated_ (the band, the typed word)
is `delete_direction().is_some()`, which excludes `purge`. A purge clears an index row for a file
already gone from both sides — putting the typed-`DELETE` gate in front of somebody for one is how a
gate stops meaning anything. The screen keys the tint on the first set and the band on the second.

**The typed word authorises this plan's deletions now (#227, landed).** `approve` used to match only
against the daemon's **current** `pending_deletions`, and at plan time nothing is pending by
construction: a deletion becomes pending only when a pass withholds it, and no pass has reached this
plan. So `approve(path)` before `sync_now` recorded nothing, and the deletion was asked about a second
time on the Deletions screen — safe, and not what the design says. A selector that matches nothing
pending now falls through to a **pre-pass approval**, pinned to a fingerprint the daemon derives from
the index itself, and `onRun` records one per gated row before it asks for the sync.

**The direction has to be named, and that is the same rule one step earlier in time.** A path alone
does not say which of the two deletions at it is meant, and #298 settled that an ambiguous selector
authorises nothing. The GUI reads the direction off the planned action (`isGated` is the one place
that pair is enumerated); a pre-approval without one records nothing and says so. A path with no
baseline record is refused rather than pinned to something that could never match — approving is the
one place this daemon stores a path a client gave it, so it also clears
`validate_relative_path_non_empty` first.

Awaited in order, one at a time: a `syncnow` that overtakes an approval is a deletion deferred, which
is the old behaviour. A failed approval is left where it falls — the pass withholds that one and the
Deletions screen has it.

**`Run it without the deletion` is drawn; `Leave it alone` still is not, and they are no longer the
same question.** G3 (#192) landed as `apply <token> --skip-destructive`: the daemon runs the plan the
user reviewed minus exactly the rows `SyncAction::is_destructive` names — the same set the screen
tints — and holds the event cursor for the dropped ones so they re-plan. The footer button names that
verb, and it is drawn **only when the plan carries a token**: a plan computed by the `--dry-run`
child (onboarding, before any daemon exists) is held by nobody and can be applied by nobody, so
there the button is hidden exactly as `06-plan.md` requires rather than faked.

`Leave it alone` stays hidden, and now for a reason of its own rather than a shared missing
capability. It reads two ways — drop this one action and run the rest (which the footer button now
does) or refuse this deletion durably (#224, which the Deletions screen's `Keep it` owns) — and a
button that means whichever the reader assumed is worse in that spot than one that is absent: it is
the escape hatch on the one screen where somebody is looking for one. Three style-gate rows record
the widths the band takes from it (`#224`); the action bar's spacer row is gone, because the button
that used to be absorbed into it is now drawn.

**No byte total exists anywhere in the dry-run surface.** `PlannedAction` carries `path`,
`destination_path`, `action`, `entity_kind`, `conflict_path` and `remote_id` — and no size. So
`files, 4.1 MB` draws as `files` on both sides of both 1040 frames (four `box.w` rows, G2 #191), and
every per-file size on the safe screen (`1.2 MB`, `2.8 MB`, `96 KB`, `2.4 MB`, `184 KB`) is omitted.
Those cost no assertion at all, which is worth saying rather than leaving to look like coverage:
every one of those rows sits inside a subtree containing an unbundled glyph, so the harness does not
compare their boxes. `new folder` and `moved` are drawable and are drawn.

**`8,431 of 12,480 files` is drawn, and its two halves still come from two different places.** The
rehearsal moved onto the daemon's main loop (#209), so `activity.files_scanned` describes _this_
walk — `activity.pass.kind === "plan"` is what makes it this rehearsal's number rather than a sync
that happened to start while the screen was open — and `index_totals.files` is the corpus size
(#207). One clause the design's fixed fraction cannot express is kept: on a **first run** the index
is empty, and a denominator of zero is not a denominator, so the line degrades to the bare
numerator rather than claiming `8,431 of 0 files`. The line is patched in place across the status
poll rather than re-rendered, because rebuilding the checking body restarts the mark's two CSS
animations from 0%.

**`Stop` is the one button in the app that masks the seam with its own fill.** `06-plan.md` calls it
out — _"`Stop` is `background:#0A0B0D`, not transparent, so the seam passes behind it"_ — and the
frame backs it up on the half the prose leaves out: the button is `position: relative` as well as
filled, which is F3's rule 3 (an absolutely positioned seam paints above a static sibling's
background _and_ its text, whatever the DOM order — the `1a Compact` bug §36 records). So the app
wears the mask through `seamMask` with `pad: null`, keeping the button's own 9px/18px padding, and
the two agree on every property. Worth writing down because it is the one place a CONTROL is a mask:
every other masked node in the design is a line of text.

**`move_remote` and `move_local` were the same sentence, and only one of them is drawn.** F7's
`OUTCOMES` gave both `moved to match Proton`, which is right for a rename that happened on Proton and
says the opposite for one you made here. Fixed to `moved on Proton to match`. Four more variants had
no wording at all — `local_delete`, `purge`, `auto_link`, `type_conflict` — and F7's note said the
words did not exist yet and inventing them would be that module doing design. That was right while
nothing rendered a plan; it stops being right on the screen that draws every row of one, because the
alternative to a chosen word is a row that names your file and says nothing about what happens to it.
`local_delete` was the worst to leave blank: it is the tinted, destructive row. All four are chosen
copy, recorded here so the deck can overrule them.

**`sortedForDisplay` is a second copy of `plan.rs::sorted_for_display`, on purpose.** That function
lives in gui-core and cannot be reached from the frontend: `run_dry_run` returns the parsed
`DryRunReport` verbatim, so what arrives over the wire is the daemon's emission order with nothing
sorted. Reusing the Rust one would mean changing what the command returns, which changes a payload
three other things parse. Both carry a comment pointing at the other.

**The gate's group is the whole footer bar, and both halves of that rule now live in one place.** The
field clears on blur unless focus lands inside `[data-delete-gate]`, and the button it unlocks is two
siblings away — so without the attribute on the BAR, tabbing from the field to `Run this sync` blurs
it, the field clears, the button disables mid-Tab and focus lands on nothing. Measured, not reasoned
about: the first version of this screen shipped exactly that and the gate could not be completed by
keyboard at all, which is the trap `deleteGate`'s own comment records from S3. The attribute and the
group's `focusout` listener are two halves of one rule (set the attribute alone and an abandoned word
stays armed; add the listener alone and the gate cannot be completed), so they are `gateGroup` in
controls.js now and `deletionGate` uses it too.

**A re-check clears the gate ALWAYS, where `06-plan.md` asks for it only when the plan changed.**
Not a decision this screen makes so much as one its own structure makes for it: a re-check goes
through the checking body, the checking body wears the four doors, and the bar holding the field
therefore does not exist while the rehearsal runs. The stricter behaviour is the safe direction — a
word typed against a plan that is being re-derived is a word typed against nothing — and building the
looser one would mean carrying the field across a footer that is not there.

**An action bar was rebuilt on every poll, and nobody noticed because every one of them was a
placeholder.** The shell patched the four doors and rebuilt the bar; the moment a bar holds a text
field that is a half-typed `DELETE` destroyed twice a second. The footer is now patched by owner
(`dom.footerOwner`) with the screen deciding what a rebuild means.

**`max-width: 496px` where `06-plan.md` writes 460.** The safe hero's sentence is masked with 18px of
padding either side, and `base.css` opts the whole app into `border-box` while the prototype does not
(§19) — so 460 of content is a 496px border box here. Capping at 460 wraps the sentence a word early.

**Eight sentences became templates, and two of them cannot be gated.** A real plan has its own
counts, its own paths and its own number of things that cannot be undone, so every sentence naming
one had to move — including `destructiveBody`, which carried the FIXTURE'S OWN PATH
(`archive/old-notes.md …`) as a literal and therefore read as a constant while being unable to name
any other file. All eight are in the copy gate's `DRAWN` table in the same commit, which is the rule
S1 wrote down after three sentences left the gate silently by becoming templates. Two cannot be:
`PLAN.destructiveLocal` (the mirror, for a deletion applied here) and `PLAN.destructiveMany` (more
than one gated deletion) are templates no frame renders, and `NOT_DRAWN` only reaches constants.
`gui/test/plan.test.js` pins both.

| screen | frame          | drawn               | Phase 1                         | gap      |
| ------ | -------------- | ------------------- | ------------------------------- | -------- |
| S4     | `5a Plan`      | `3` `files, 4.1 MB` | `3` `files`                     | G2 #191  |
| S4     | `5a Plan safe` | a size on every row | the row without its size        | G2 #191  |
| S4     | `5a Plan`      | `Leave it alone`    | hidden                          | #224     |
| S4     | `5a Checking`  | a 522px window      | a 520px column in a 1040 window | decision |

### 77. S5 · the activity screen

**This is the screen the daemon says least for, and most of the work was deciding what not to draw.**
Eight blocks the frames draw have no Phase-1 source, and six of them needed an issue filing before
this screen could be honest about them. The rule throughout is `IMPLEMENTATION-PLAN.md` §4's — omit
the clause, never fake it — applied at the scale of whole blocks rather than trailing clauses,
because on this screen a number is a claim about whether someone's files are safe.

**One block went the other way, and it is the first time that has happened.** The F9 fixture recorded
the never-synced list as unbuildable and explained exactly why: matching an exclude glob against the
index cannot work, since the selective-sync invariant filters excluded paths out of the local scan,
the remote listing _and_ the base index — _"counting them means walking the filesystem, not reading
the index."_ That is precisely what C2's `skip_rule_usage` shipped one PR earlier. So the band and
the dialog's first group are live data, and `RuleUsage.samples` gained a size per file (the dialog
draws `path · size` rows and the walk already stats every file it counts).

**The lookup is an exact relative path, and the frame draws a search.** `7a File lookup` resolves the
query `spec.md` to `docs/spec.md` with `1 match` beside it; `path_sync_status` opens the index at the
path it is given, and none of the 23 commands lists or searches local files. So a bare name that is
not at the sync root misses, and the miss draws `14-behaviour-and-state.md:130`'s own sentence. Two
consequences: the count is only ever 0 or 1, so the plural arm of `ACTIVITY.matches(n)` is
unreachable, and the frame's own query is not reproducible by the shipped screen. G21 #234.
**Closed 2026-08-13 by `search_files` — see §94.**

**Four of the five lookup verdicts are undrawn, and they are the screen's front door.** `path_sync_
status` answers `synced`, `modified` or `conflict`, reports `tracked:false`, and marks a directory
through `entity_kind`. The frames draw `synced`. The other four are reachable from the first thing
anyone types, so each gets a verdict, a hexagon state and a deck sentence — S5's wording, not the
deck's, except `noMatch` which `14-behaviour-and-state.md` specifies verbatim. All eight strings are
exempted in `copy-gate.mjs` with that reason and pinned by `gui/test/activity.test.js`, including the
arm nobody designed for: an **unrecognised** `sync_status` must never come back as the settled mark
or the safe words. A fourth value is a thing the engine could grow, and the reassuring answer is the
dangerous one.

**The lookup field is a `contenteditable` span, and an `<input>` was tried first.** The gate records
a tag and never compares one — that is why every footer door is drawn as a `span` and emitted as a
`<button>` — so an input styled to the span's numbers looked like the obvious answer. It is not:
Chromium's user-agent stylesheet sets `overflow: clip !important` on text inputs, and a UA
`!important` outranks an author `!important`, so an `<input>` computes `clip` where the drawn span
computes `visible`, forever. Verified with a probe rather than assumed. `overflow` is an asserted
property, so the tag choice leaks out of _recorded but never compared_ into a real mismatch on this
screen's primary control. The two alternatives were worse: excluding `overflow` from the gate would
absorb an app decision into the harness (the width/height exclusions are for things no app choice
controls) and would quietly stop asserting it on every future input; unmapping the node would trade
one property for colour, both font states and the flex sizing that distinguishes the field's two
states. `contenteditable="plaintext-only"` is WebKit's own, keeps the drawn tag, and types — at the
cost of a `::before` placeholder, a swallowed Enter, and stripped newlines on paste.

**`5a Checking` draws no lit door, and the app lights one.** `02-shell.md:42` states the rule without
exception — _"the active one is `#F2F4F7`"_ — and all three S5 windows draw it. `5a Checking` is the
plan screen, so `Plan a sync` should be lit, and the frame paints all four unlit. S5 is what surfaced
it, because until now no mapped frame had a door that COULD be lit: `2a` is the root and `3a`/`4a`
are overlays, whose frames correctly light nothing. The app follows the prose and `door` is
undeclared in `planFids`. Deliberately NOT a `known-deviations` row: that file's bar is a missing
capability with an open issue, and this is a drawing mistake. Eight `#221` rows that rode on those
door nodes were removed with them; `#221` is still recorded on the same frame through `div[1]` and
`div[1]/div` — **settled by §103** (2026-08-17), which moves those eight rows to `decision: true`
alongside `3a Conflicts cleared`'s and `4a Empty`'s: the shell does not resize for any of the three.

**Every dialog fid slot is prefixed, because a slot name is resolved by NAME.** A dialog floats over
a body, so both screens render and both call `fid()`. A `hexagon` declared for `7a File pending`'s
48px mark is stamped by whichever hexagon the screen behind it drew first — the 168px main-screen
mark, or the quiet tab's 52px settled one — and the result is reported as a size mismatch on a screen
nobody was looking at. That is the failure `activeRoute`'s own note describes, one layer up. The
three dialog fixtures also name their route, so the thing behind the scrim is the screen the dialog
is actually opened from.

**Six defects the frames found by being rendered against, none of which any earlier gate could see:**

- **A mounted dialog could never change.** The dialog layer's identity check on `dialogRoute` was the
  only thing that rebuilt one, so `6a Details` — eight live counters — froze at whatever the reply
  held when it opened. Now keyed on a content signature, replacing only the children below the head
  and carrying focus across by position, so `Copy all` survives a counter moving under it.
- **`status_history` is oldest-first and the frame draws newest-first.** `daemon.rs` pushes each pass
  and drains the front. The summary's `recovered` argument reads the LAST entry for the same reason.
- **`.row-pass.is-failed` is a block that never gave back the flex row's `gap` and `align-items`.**
  Latent since F7 — `6a Activity passes` is the only frame that draws a failed pass, and nothing
  asserted it until S5 mapped it.
- **`.dialog-head-wide`'s shorthand padded the bottom 22px** where `7a Never synced` records none,
  pushing every row below it down by that much. `.dialog-subtitle` was also missing its `margin-top`
  and `line-height`.
- **The lookup sub-line rendered a live clock where the fixture pins a literal.** `clock.js`'s rule
  says an absolute time is pinned as a string because an epoch formatted as `14:32` moves with the
  timezone and across midnight; nothing had read `ui.clock` before, so the sentence's width landed
  wherever the hour put it — green at some hours and red at others.
- **`detailOf` handled three of fourteen counters,** so the frame's own `4 brought here · 1 move
followed` row rendered its `local_moves` as nothing. Invisible to every gate: the detail span is
  not individually mapped, `assert.mjs` does not compare text, `.pass-detail` is a fixed 230px, and
  the copy gate does not walk `format.js`. A pass that did work and reports nothing is the wrong
  failure for this screen. The remaining counters are covered in the frame's own register (`1 move
followed` is drawn); `purges`, `auto_links` and `type_conflicts` are deliberately left out, since
  none of them moves or removes a file and this line is about what happened to files.

**Seventeen ACTIVITY templates joined the copy gate's `DRAWN` table, and fifteen had never been
checked by anything.** The whole block's templates were absent while its constants were green, so
every counted sentence on the busiest screen in the app was unasserted. Two more became templates in
this commit — `passes.summary` (every number in it is live) and `neverSyncedSub` (the two group
counts) — and landed in the same commit, which is the rule S1 wrote down the first time a sentence
left the gate silently by becoming a template. This is the third time it has come up. Four templates
were also ungrammatical at one (`1 files are never synced`), all four reachable.

**`neverSyncedSub` needed a lower-case cardinal, and `cardinal` was documented as sentence-start
only.** The drawn sentence capitalises the first clause and lower-cases the second — _"Two match a
rule you wrote; two can't be synced at all."_ — so it needs both forms in one sentence. A `register`
argument rather than a caller-side `.toLowerCase()`, since above ten `cardinal` already hands back to
`count()` and the register only ever touches the spelled forms.

| screen | frame                | drawn                                    | Phase 1                                                     | gap          |
| ------ | -------------------- | ---------------------------------------- | ----------------------------------------------------------- | ------------ |
| S5     | `6a Activity passes` | a twenty-bar duration chart              | the whole card omitted                                      | G16 #229     |
| S5     | `7a Activity quiet`  | `12,480` `files · 41.2 GB`, both sides   | both numeral rows omitted                                   | G7 #207      |
| S5     | `7a Activity quiet`  | `next full check in 4m`                  | the sub-line omitted                                        | G4 #193      |
| S5     | `7a Activity quiet`  | `Last things to move`, head + three rows | the block's footer row alone                                | G17 #230     |
| S5     | `7a Activity quiet`  | `4 files are never synced`               | ~~the rule-matched count alone~~ both groups (§98)          | G19 #232 ✔   |
| S5     | `7a File lookup`     | `This file's history`, four rows         | the `linked · id` line alone                                | G1 #190      |
| S5     | `7a File lookup`     | the query `spec.md` → `docs/spec.md`     | ~~an exact relative path~~ a search (§94)                   | G21 #234 ✔   |
| S5     | `7a File lookup`     | `received 14:32` on the Proton card      | ~~the clause omitted~~ drawn on an upload (§98)             | G20 #233 ✔   |
| S5     | `7a File pending`    | a 3px bar at 41%                         | no track at all (§63)                                       | G2 #191, #98 |
| S5     | `7a Never synced`    | `Can't be synced`, two rows              | ~~the group omitted~~ drawn; the link's target is not (§98) | G19 #232 ✔   |
| S5     | `6a Details`         | `Open the system log`                    | ~~omitted~~ drawn; a journal snapshot (§97)                 | G18 #231 ✔   |
| S5     | all three            | `Open folder`, `Open on Proton Drive`    | ~~omitted~~ drawn; the Drive app, not the file (§97)        | G18 #231 ✔   |
| S5     | `5a Checking`        | four unlit doors on the plan screen      | `Plan a sync` lit, per `02-shell.md:42`                     | —            |

**`ACTIVITY.nothingRecent` stays undrawn, and it looks like it should not.**
`14-behaviour-and-state.md:129` gives it as the empty state for `Activity › files` — _"Nothing has
moved in the last hour."_ — and the shipped screen has no `Last things to move` rows, so that state
is now the standing one rather than an edge case. It is still not rendered: the sentence is a CLAIM
about the last hour, and the gap that removed the rows (#230) is precisely the absence of any
per-file record to make it from. The app cannot know whether it is true. `quietIsNormal` is the
frame's own sentence, says nothing the daemon has not reported, and is what the row keeps.

**The pending dialog's trigger is the only one the data supports.** No frame and no doc says how
`7a File pending` is reached, and `routes.js` had no id for it. `7a File lookup` and `7a File
pending` are the same lookup in two states — a file that is settled, and a file that is moving right
now — so looking up the file the daemon is currently transferring is what tells them apart. Nothing
else could: exactly one transfer is ever in flight — `execute_plan_and_commit` is a sequential loop,
which is why `SyncActivity.transfers` has one `active` row and a queue behind it (§63c) — so a lookup
for any other moving file cannot reach this state. It is latched, because the condition stays true for as long as
the transfer runs and Esc would otherwise reopen it on the next render, and it closes itself when
the transfer ends rather than degrading to the not-built-yet placeholder.

**The lookup is debounced by 180ms, which is a correctness fix rather than a nicety.**
`path_sync_status` is synchronous on the Rust side and its own module header warns it "can hold the
loop for its full 3s index busy timeout". One index open per keystroke queues behind the daemon's
own writer, and a 20-character path is 20 of them whose answers the latest-wins guard then throws
away. The field repaints on the keystroke and the index is asked afterwards; the two are
deliberately not coupled.

**A MAPPED FRAME CAN RENDER ALMOST NOTHING AND THE GATE STAYS GREEN, and S5 is where that finally
cost something.** The never-synced band and the whole `7a Never synced` body rendered EMPTY through
four separate causes, and every gate passed: `assert.mjs` compares nodes the app has stamped, so a
block that renders nothing stamps nothing and is simply not compared. It was found by dumping the
`data-fid` attributes per frame — `7a Never synced` had 2 where it should have had 7 — not by any
check.

`assert.mjs` now reports, per frame, the slots its fixture DECLARES that the running app never
stamped. `check-fixtures.mjs` already fails on the complement (a declared slot whose key exists in
no frame — how the dead `hexRect`/`hexNumeral` declarations were found); this is the other half, and
it is invisible to that gate because the key is real and merely unreached.

**S5 made it a report rather than a failure — §80 overturned that, and the rest of this paragraph
is the argument it overturned.** The reasoning was the one behind the unmapped-frames line: it is
true of shipped screens too. `2a Syncing` and `2a Needs you` declare `transferTrack`/`transferFill`
for a progress bar that is unreachable by construction (§63, #98), and five compact frames declare
`meta` and `action` for panel states they do not draw. Listed every run so that "the gate is green"
is never confused with "the gate looked at anything".

What §80 found is that those two halves are not alike: the five compact frames were never a finding
at all — their frames draw no such node — while the progress bar is a real omission that belongs on
a list with an issue against it. Sorting them is what let the residue become binding.

Acting on S5's own entries took the screen from 49,299 assertions to **51,743**, and the newly
compared nodes were not all correct: thirteen further mismatches surfaced the moment they were
stamped, every one of them shared dialog CSS carrying a value that differs per dialog —
`.dialog-head-compact` padding its bottom where the frame gives none (the same defect the wide rung
had), `.dialog-title` taking the flex that belongs to `.dialog-headings`, and `gap` / `align-items`
/ `overflow` stated once in `dialog.css` where `6a Details` and `7a Never synced` disagree on all
three. What repeats is in the stylesheet now; what differs is inline at the call site, which is what
the measurement said in the first place.

**The ✕ is a fixed 26x26 square, and that is a robustness fix.** `✕` (U+2715) is outside the bundled
font subsets, so its advance comes from whatever the machine has installed — left to size itself the
button came out 1.94px narrower here than on the prototype's machine. The error does not stay in the
button: `.dialog-headings` is `flex: 1` beside it, so the title and sub-line under it inherit it, and
`boxComparability` cannot see that — its taint reaches a node's PARENT, and these sit two levels
below the flex container holding the glyph. Pinning the drawn size makes the head's layout
independent of the font rather than recording a deviation for it.

**`Both sides agree` was rendered unconditionally, which is a false all-clear.** `7a Activity quiet`
draws the settled hexagon over the strongest claim the app makes, and the screen rendered it in
every daemon state — paused, syncing, unreachable, first run. `copy.js` already records the
identical failure one screen over: the sign-in-expired hero exists because a state "that fell
through to `Everything is up to date` would be a false all-clear on a daemon that cannot reach
Proton at all". Same sentence class, same mistake, a screen later.

The verdict block is now drawn only when `derive_state` reports `idle` AND a pass has been recorded
— a daemon that answered, has nothing outstanding, and has a moment for the claim to be true at.
OMITTED rather than replaced in every other state: no frame draws this screen outside idle and the
deck has no sentence for one, so inventing a verdict would be the second mistake. The seam and both
sides stay, since their sub-lines are already gated on having something true to say.

Found by sweeping the diff for the shape of two review findings rather than fixing them where they
were reported — an unrecognised status that failed closed on the mark and not on the words, and a
dialog that read an absent reply as a negative one. Both were doctrines this codebase already holds
(`countersUnknown`, `dash()`, the all-clear note above) applied in one place and not the next; the
sweep is what turned that into a rule rather than three separate fixes.

## The settings screen (S6)

## 78. Four tabs, three drawn, and the largest single deviation in the build

`08-settings.md` specifies four tabs and the frames draw three of them. `8a Settings` is Folders,
`8a Skip rules` is What to skip, `8a Deletions tab` is Deletions, `8a Schedule monthly` is one panel
of Folders in its other state, and `8a Save refused` is a dialog over any of them. **Advanced is
specified in prose and drawn nowhere.**

### 78a. A crop is a re-render, not a cut-out — and its boxes are not comparable

`frame-classes.mjs` has said since F8 that a crop's own width is "an artefact of how it was drawn".
It is more general than that, and `8a Schedule monthly` is the proof: it is the SAME panel
`8a Settings` draws, at `padding:18px 20px` against the window's `13px 18px`, with an 18.75px
sub-line against the window's 18.125px. **Two frames of one panel disagree with each other**, so
neither can be the box the app owes — and the disagreement is not confined to the outer node, since
a 600px re-render draws its children 546 wide where the 1040 window draws them 976, and text wrapped
at 546 is twice the height it is at 976.

So `OWES_BOX(kind)` now skips the box comparison for a crop, whole. **Styles are still compared in
full**, and that is what these two frames are actually evidence of: the radio card's tint, its ring,
its badge and all three of its body colours came off `8a Deletions tab` and are asserted against it.

The rejected alternative was a 546px column invented inside a 976px tab so a crop's arithmetic would
come out — a screen built to satisfy a measurement rather than a design.

One node stayed unmapped for a related reason. `8a Deletions tab`'s `deletion_policy` key line
(`div[3]`) is positioned by `margin-top:auto`, and a computed margin resolved by `auto` is a **used
value**: 72.375px in a 520-tall crop, 172.5px in the 764-tall window. Same artefact, arriving
through a property `OWES_BOX` does not cover. The line ships as drawn; it is the crop that cannot say
where it sits. `8a Schedule monthly`'s head row (`div[0]`) is unmapped on the same footing — the two
frames give it different gaps.

### 78b. The schedule panel keeps its shell and changes its subject

**This is the largest single Phase-1 deviation in the design-v2 build**, and the issue that scopes
S6 says so in advance.

The frame draws a whole full-sweep schedule: a Weekly/Monthly segmented control, seven day chips, a
time stepper, and the key line `full_scan_schedule · weekly sun 03:00`. There is no
`full_scan_schedule` key, no scheduler in the daemon and no command that returns any of it (G4,
[#193](https://github.com/osirison/proton-drive-sync-engine/issues/193)). `events_full_scan_every` is
the nearest real thing and it counts _passes_, not days, and defaults to off.

`IMPLEMENTATION-PLAN.md` §4 says to present `scan_interval` in plain language inside the same panel
shell. So the shell, the head row and the divided control row are the frame's, and the subject is
Phase 1's — **including the title**. `Compare everything, top to bottom` over a timer that (with live
updates on) schedules an _incremental_ pass would be a false claim about what happens to someone's
files, which is the one thing this screen may not do. The panel now reads `Look for changes on a
timer` over `scan_interval_secs`, with the stepper in the row the day chips were drawn in.

`assert.mjs` does not compare text, so the retitled block still asserts every style and its own box:
the three `box.w` rows in `known-deviations.mjs` are the head row spreading into the 155px the
segmented control and its gap were holding, and nothing else.

### 78c. Two keys, two different answers, and the difference is which side is wrong

`8a Settings` draws `event_driven_reconcile` under the live-updates toggle. **That key does not
exist** — the engine's is `events_driven`, and `14-behaviour-and-state.md:25` says so in as many
words. The app draws the real key and the node is left **unmapped**: this is the prototype being
wrong rather than the app being unable, which is neither a mapped node nor a `known-deviations` row
(the bar there is a missing capability with an open issue). Same call `5a Checking`'s unlit doors got.

`8a Deletions tab` draws `deletion_policy · applies to both directions`, and **that one ships as
drawn**. It also names a key the daemon does not have — §68 — but it names the _policy the two
`[delete_approval]` booleans express_, which is what a person choosing between three cards is
setting. G5 ([#194](https://github.com/osirison/proton-drive-sync-engine/issues/194)) would mint it
natively. The two lines get opposite treatment because the frame is wrong about one and early about
the other.

### 78d. Everything above the footer is the saved config; the footer is the staged edit

`8a Skip rules` draws a removal staged but not saved, and F9 recorded the frame as internally
inconsistent: `video-raw/**` is in the list at full opacity while the footer says that rule was
removed, and it is counted inside `hiding 4 files, 3.1 GB in total`. The fixture left the call to S6.

It is not inconsistent — it is a rule. **Every count on the tab was measured against the config on
disk**, so a list that dropped the row while the total still counted it would be a screen disagreeing
with itself. The rows are the saved config, the footer is the diff, and the amber cost line is where
the news is.

A staged ADDITION is the one exception and barely one: an added rule has no measured row to leave
alone, so it appears with `Not saved yet` where its counts would be. Without it, `Add` would look
like a control that does nothing — the removal at least turns the footer amber.

The cost line reads `unique_files`/`unique_bytes` (§69b) and only fires for a **single** removal:
the deck's sentence begins `One rule removed`, and there is no plural form to reach for.

### 78e. The rule sub-line's discriminator, and the cell that fails closed

§69a left this to S6: the frame draws two of the five answers a `RuleUsage` can produce, and live
data has no byte discriminator. The rule is the **folder anchor** plus whether the samples are the
whole set:

| `files`    | `folder_exists` | drawn                                                                               |
| ---------- | --------------- | ----------------------------------------------------------------------------------- |
| **absent** | —               | `Checking…`, no second line — the walk has not answered for this rule               |
| > 0        | `null`          | `Skipping 2 files right now` + the paths — **only when `samples.length === files`** |
| > 0        | `true`/`false`  | `Skipping 2 files, 3.1 GB` + `the folder still exists…` when it does                |
| 0          | `false`         | `Matching nothing` + `safe to remove`, at `opacity:.62`                             |
| 0          | `true`          | `Matching nothing` + `the folder still exists…`, **not dimmed**                     |
| 0          | `null`          | `Matching nothing`, no second line                                                  |
| —          | `error`         | `Couldn't be checked` + the walk's own words, in mono                               |

The first row is the review's (§78j): `files ?? 0` collapsed "not measured" into "measured zero", so
every rule read `Matching nothing` for the length of a local-tree walk and permanently after a failed
one. Three of the seven are the point. The samples clause exists because the command caps samples at four
(`MAX_SAMPLES`) and a list of four under `Skipping 50 files right now` reads as the full set. And
**`safe to remove` is never said of a rule that is hiding something**: it needs nothing matched AND
the folder known gone, because removing a rule that still matches files starts syncing them, and
this is the sentence someone acts on without checking. `gui/test/settings.test.js` pins the whole
table, including the three cells no frame draws.

`hiding N files` also has a hedged form. `skip_rule_usage` returns
`unreadable_directories`/`unreadable_entries` precisely so the tab does not present a floor as a
fact; with either above zero, every number on the tab is a lower bound and the line says so.

### 78f. The refusal Phase 1 can actually produce

`write_config` refuses on `ConfigDoc::validate` — a serde/TOML check against `FileConfig` that never
contacts Proton Drive. So it cannot know a remote folder is missing and cannot say `That folder
doesn't exist on Proton Drive`; nor is there a command behind `Create it on Proton Drive`. Filed as
G22 ([#236](https://github.com/osirison/proton-drive-sync-engine/issues/236)).

What ships is the generic title, the sentence `08-settings.md` calls the important one — _"Nothing
was saved — your old settings are still running."_ — and the daemon's reason in mono with the
`config would be rejected by the daemon: ` prefix stripped, because that prefix is the GUI's sentence
about the daemon's words and the body already says it.

**And a save that succeeds restarts the service** ([#320](https://github.com/osirison/proton-drive-sync-engine/issues/320)).
There is still no config-reload path in the engine — no SIGHUP handler, no watcher (§68) — so a
written file and a running daemon disagreed until somebody pressed a second button, and that gap was
reachable by an ordinary sequence: change the sync folder, open Plan, and the preview describes the
file's pair while `Run` executes the daemon's. The save now asks the daemon to shut down and starts
it again (`restart_service`, `ControlCommand::Shutdown`), which makes the mismatch unreachable
rather than reporting it.

Three consequences, all of them drawn nowhere:

- **The interruption is announced before the click.** While a **daemon-config** change is staged and
  a **counted** pass is running, the bar reads `Saving restarts the sync service, which stops the
sync that is running now.` — above the cost line, which is the only ordering this decision
  settles. Both adjectives are load-bearing and neither was there at first
  ([#335](https://github.com/osirison/proton-drive-sync-engine/issues/335)): a staged notification
  policy writes `gui.toml` and restarts nothing, and a plan-only rehearsal claims `syncing` without
  moving a file, so the sentence twice named an interruption that would not happen.
- **A save never STARTS a service that was not running.** `restart_service` takes `only_if_running`
  for the save path: a stopped daemon has nothing to interrupt and nothing stale to correct, it
  reads the file when it next starts, and a save is not a request to begin syncing. Only an
  _observed_ absence counts — a probe that could not tell does nothing and says so, rather than
  asserting the daemon is down (#335).
- **Every ending is loud, and there are five of them** (#335). `RestartOutcome` is a typed,
  internally-tagged answer on the command's Ok payload, one sentence each: it restarted; it was not
  running so nothing was started; the start failed so **nothing is running**; it never stopped so the
  **old** process is still up on the old settings; or it could not be told apart so nothing was done.
  The last three keep `Restart it now` in the second slot until the state they name is over — the
  slot holds `Discard changes` otherwise, including after a settled save, which has no action of its
  own. #320 typed two of the five and let three collapse into one `Err(String)`, which is how a save
  that stopped the daemon and failed to start it drew `It is still running the old settings`.

### 78i. The one node this screen adds, and why it is not on a mapped one

The window is fixed at 764px and cannot grow. A config with a dozen exclude rules pushes the add row
and the `.sync` note straight through the footer — which `02-shell.md` calls "a real bug found twice
during this design", and which measured 1040×1158 with thirteen rules before it was fixed.

**The scroll cannot go on any node the frame draws.** All 22 in-scope 1040 frames leave their content
`overflow: visible` (only `6a Activity passes` sets `hidden`), `overflow` is an asserted property,
and every node on this tab is mapped. `shell.css`'s `.content-region` takes the other option — it
sets `hidden` and opts a genuinely long list into `.is-scrollable` — but it does so on a node no S6
frame maps.

So the rows sit inside a wrapper the frame has no node for, capped at five rows (`59px` each, against
about 325px of space between the list head and the add row). Nothing stamps the wrapper, so the gate
never sees it; the rows keep their own keys, and the first row keeps the 11px the frame records on
the ROW rather than moving it to the wrapper.

**A cap with `+n more` was the other option and it is wrong here.** That is right for the main
screen's transfer rows, which are a report; these rows each carry the only `Remove` button that rule
will ever have, so hiding the twelfth would make it unremovable from the screen that exists to remove
it. The include list on Advanced is capped the same way, in a shorter panel.

### 78g. Two commands, and why they were not C-items

`commands.rs` says screens never add to the command surface and that capability tasks are how it
grows. S6 added two, and they are recorded there rather than smuggled in:

- **`resync`** — `Sweep now` is a _full-tree_ walk and `sync_now` is not one. `ControlCommand::Resync`
  has been in the daemon since #160 and no command exposed it.
- **`choose_folder`** — a native folder picker behind the same facade as everything else, so `api.js`
  stays the frontend's only backend surface and no capability JSON grants the webview a file dialog
  of its own. `tauri-plugin-dialog` is registered for the Rust side alone.

Both are five lines over machinery that already exists; the alternative was two more dead buttons of
the kind #224 and #227 already record. A screen that needs _data_ still files a C-item.

### 78h. Four primitives corrected by the first screen to draw them

`radioCard`, `toggle`, `stepper` and `dayChips` were all written by F5 from prose and none had a
consumer until now. Every one of them was wrong somewhere:

- **`radioCard`'s tree.** F5 wrote `card > [ring, body > [title, text]]`; the frame draws
  `card > [head > [ring, title, badge], text]`. The ring is inside a flex head row and the body is
  that row's sibling, indented past it by a 26px padding rather than by nesting.
- **The unselected ring is 1px, not the 1.5px `08-settings.md` gives it.** The frame wins, as it does
  for the 13-to-6 footer split (§40). The selected ring has no inner dot either — the drawn dot IS a
  4px border with a 7px well of the card's own surface showing through.
- **`toggle` had a border and the frame has none**, and its knob is 20px rather than 18. The knob now
  moves by `left` rather than a transform, because an absolutely-positioned node reports used values
  for `left`/`right` and a transform moves the pixels while leaving both where they started. A
  `<button>` also has to put back the `font`, `color` and `text-align` the UA sets, all three of
  which are asserted properties and none of which a drawn `div` has.
- **`stepper`'s glyph is `＋` (U+FF0B), at 13px** — the icon rung's 15px is the `⋯` menu's size.
- **`dayChips` is 42px at a 5px gap**, not the pill row's 44/8. S6 does not render them (the schedule
  is G4), so those two numbers are waiting for whoever builds it rather than checked by anything.

Three tokens were minted with them: `--destructive-card-bg`/`--destructive-card-border` (.04/.30
against the base .06/.38 — the per-site alpha step §52a measured across the bands) and
`--destructive-ring`, the one place the design tints an inert hairline, drawn opaque because a 1px
ring at .3 over a .04 fill would be invisible.

| screen | frame                 | drawn                                                          | Phase 1                                        | gap        |
| ------ | --------------------- | -------------------------------------------------------------- | ---------------------------------------------- | ---------- |
| S6     | `8a Settings`         | the Weekly/Monthly control, day chips, time stepper            | the panel shell over `scan_interval_secs`      | G4 #193    |
| S6     | `8a Settings`         | `12,480 files, 41.2 GB in here today`                          | the merge warning alone                        | G7 #207    |
| S6     | `8a Settings`         | `A full check of all 12,480 files as a safety net`             | a sentence about the timer                     | G7 #207    |
| S6     | `8a Settings`         | `Takes about 4 minutes … Last one 2 days ago`                  | what is true every time                        | G24 #238   |
| S6     | `8a Settings`         | `event_driven_reconcile`                                       | `events_driven`, the key that exists           | —          |
| S6     | `8a Skip rules`       | `added 14 Jul` on a rule                                       | the folder clause alone                        | —          |
| S6     | `8a Skip rules`       | the unsyncable panel and `See them`                            | ~~omitted~~ drawn from the standing list (§98) | G19 #232 ✔ |
| S6     | `8a Schedule monthly` | the whole monthly variant                                      | the panel head alone                           | G4 #193    |
| S6     | `8a Save refused`     | `That folder doesn't exist on Proton Drive`                    | a generic refusal title                        | G22 #236   |
| S6     | `8a Save refused`     | `Create it on Proton Drive`                                    | omitted; `Go back and fix it` stays            | G22 #236   |
| S6     | _(not drawn)_         | Advanced: socket path, log level, conflict suffix, index reset | named as not writable yet                      | G23 #237   |

### 78j. What the review found, and the shape it was

Four independent reviewers over the S6 diff produced 32 findings; 24 survived an adversarial
refutation pass. **No gate caught any of them** — the fifth screen in a row for which that is true —
and they fall into three shapes, none of which a fidelity harness can see.

**Silence where a control failed.** Five findings, one cause. `resync`, like every status command,
**resolves** with a socket failure folded into its payload rather than rejecting (`commands.rs` says
so in as many words), so `await api.resync()` inside a `try` is a `catch` that never fires: against a
stopped daemon, or one older than `ControlCommand::Resync`, `Sweep now` did nothing at all and said
nothing at all. `restart_service` does reject — and its reason went into `settingsError`, which only
the refusal dialog reads and only `saveSettings` opens, so a failed restart was equally silent while
the bar still said the save had landed. Both now report into the bar's own sentence, and both have a
busy state: PR #140 filed this exact shape once already.

**A screen answering for a file it could not read.** `read_config` rejects an unparseable config and
`refreshConfig` swallowed it, so `configInfo ?? {}` drew an empty, valid config — blank folders, live
updates on, a five-minute timer, and a **deletion-policy card selected that is not the one running**.
The failure is now recorded (`configError`), the tabs carry a line saying the settings on screen are
not the settings in force, and the Deletions tab selects no card until the file has actually been
read. Same class as §68's fourth combination: this screen does not guess at a safety policy.

**Unknown drawn as zero, twice.** `ruleEffect` read `rule.files ?? 0`, so a rule the walk had not
measured — the whole duration of a local-tree walk, and permanently after one fails — rendered
`Matching nothing`, one line above `safe to remove`. And `configUpdate` compared a staged value
against `null` for an absent key, so clicking the deletion card that was **already selected** marked
the screen dirty and wrote two keys the file never had. Both are this project's own rules broken
inside the screen that states them: "unknown is never zero", and a footer promising that saving
writes only what you changed.

Two more worth naming. **Focus**: the body is rebuilt on every poll and only the five text inputs
were restored, so tabbing to any of the other twenty-one controls lost the keyboard to `<body>`
within two seconds — measured, not argued. Every control now carries a `data-sfocus` id, the restore
scans for it, and the same attribute is what lets `Go back and fix it` put the caret back in the
field it told you to fix. **A save in flight**: `settingsEdits = {}` on the way back discarded a
keystroke typed while the write was running and then reported it saved; the map is now cleared only
if it is still the one that was sent.

**And two fixes went into the engine's side of the wall.** `ConfigDoc::validate` was a serde parse,
which passes `local_root = ""` and `exclude = ["["]` — both fatal at
`config::validate_runtime_config`, i.e. after the GUI has said "Saved" and the daemon running the old
settings is gone. It now makes the daemon's own two checks, so those configs are refused at the write
and surface through `8a Save refused` with the daemon's wording, which is the path the frame exists
for.

**And four more from the review bot's SUPPRESSED block, over two passes — which is where this
repo's real findings keep turning up (13/13 before this PR, 15/15 after).** They arrive collapsed
behind a `<details>` and never as inline comments, so they have to be expanded by hand every time. `choose_folder` folded a task-join failure into `None` with `unwrap_or(None)`, making a
picker that could not open indistinguishable from one somebody closed — the same silence as
`Sweep now`, one file over; it now returns `Result<Option<String>, String>` and the bar says which
happened. And `.radio-card.tone-destructive .radio-ring` was three classes to the selected rule's
two, so it won the cascade on `border-color` and drew the SELECTED `Never ask` dot in `#6B3A3A`
instead of white. No frame draws that state — `8a Deletions tab` selects the first card — so nothing
could have compared it, which is the same blind spot as everything else in this section.

The third pass found the other two, and both are a promise the code had stopped keeping. The
picker's `into_path()` failure still folded into `Ok(None)` — i.e. into "dismissed" — one commit
after the doc comment above it started promising the opposite. And `loaded` was `configLoaded`
alone, so a config that PARSED once and stopped parsing later left a stale `configInfo` behind a
true flag: the screen kept a deletion-policy card selected from the last good read, underneath the
banner saying the file could not be read. Answering for a file and disclaiming it in one breath.

The eight refuted findings were: the per-visit reset (deliberate, and documented at the function),
the toggle's transition on first paint (§30's own rule), `intervalLabel(0)` (unreachable —
`MIN_INTERVAL_SECS` clamps), the floor hedge on a row (the header carries it), the zero-cost removal
(78d), the content region's scroll (78i, already fixed), `OWES_BOX` (78a), and a claim about the
daemon still running.

## The onboarding takeover (S7)

## 79. Two steps, three dialogs, and a takeover that cannot survive its own success

`09-onboarding.md` describes one flow: choose two folders, look at what the first merge would do,
watch it happen, agree to two-way deletion. The five `9a` frames are three window states and two
dialogs, and the shell has one slot for each — except that the sequence itself has nowhere to live.

**The takeover holds steps 1 and 2 only, and the two post-merge frames float.** `routes.js`'s latch
releases on any reachable daemon state, which is by design: the takeover exists so a machine with no
daemon is not stranded on the unreachable screen, and its exit condition is the daemon coming up.
`Start the first sync` starts the daemon. So the moment the flow succeeds, the latch opens — and
`9a First sync` and `9a Consent` are both drawn AFTER that, against a `running` and a `paused`
daemon respectively (the F9 fixtures pin exactly those states, and say so).

Two answers were possible: a second latch the daemon state cannot release, or the two surfaces as
dialogs over whatever the released latch left behind. The frames settle it — both are drawn at 600px
with their own chrome, not as 1040 windows — so they are dialogs, and `app.js` drives all three from
`onboardingStage` rather than through `openOverlay`. That also gives the third one what it needs:
`9a CLI missing` is a dialog **over** the takeover, and the line that resolved the dialog layer used
to null itself out whenever the takeover was open.

None of the three is closable and Esc reaches none of them, because none is in `dialogOverlay` at
all. The cost is that `closeOverlay` cannot be used to dismiss them; the flow's own buttons do it.

### 79a. `Syncing stays paused until you agree.` is made true rather than drawn

Nothing starts a daemon paused. `start_service` starts it and it syncs; the consent screen then
claims it is paused, which would be a false statement about someone's files — the thing the schedule
panel one screen over (§78b) changed its own title rather than say.

So the flow makes it true: when the merge's pass completes (`reconcile_seq` has advanced past the
value it held when `Start the first sync` was pressed, and `syncing` is false), the app pauses the
daemon and opens the consent dialog. `Start syncing` resumes it. Leaving without agreeing leaves the
daemon paused, which is exactly what the sentence promises, and the main screen behind offers
`Resume`.

The counter is load-bearing and not decoration: a status reply arriving before the daemon's first
pass begins reads as a finished merge without it, and the consent dialog would open over a merge
that had not started.

### 79b. `See all 471 actions` is a derivation, not a field

The frame draws `See all 471 actions` and `3 files can't be synced` on the same screen, and both are
true: `SkipUnsupported` **is** a plan row, so `PlanSummary::total` is 474 and the button names the
471 that will actually happen. `actionsThatHappen` is `total − skipped_unsupported`, in one place.
Reading `total` straight draws 474 and the two numbers stop agreeing with each other.

(The button itself is not drawn — see 79e — but the derivation is, because the same question is what
the fact row answers.)

### 79c. What Phase 1 cannot say, and what it says instead

Every row here is a clause or a node the flow cannot source. All are in `known-deviations.mjs` with
the exact measurement, so the day the capability lands the build fails until the row is deleted.

| Drawn                                                    | Phase 1                                                 | Why                                                                                                                                | Issue |
| -------------------------------------------------------- | ------------------------------------------------------- | ---------------------------------------------------------------------------------------------------------------------------------- | ----- |
| `341 files / 2.1 GB` per card                            | no stats row                                            | nothing counts files or bytes under a _candidate_ folder — the pair is not indexed yet, because this is the screen that chooses it | #240  |
| `Signed in as you@proton.me · 39.1 GB of 500 GB used`    | omitted                                                 | the daemon reuses the CLI's keyring session and never sees an address or a quota                                                   | #241  |
| `files · 1.4 GB` / `files · 38.4 GB`                     | `files`                                                 | no level of the dry-run surface carries a size                                                                                     | #191  |
| `Needs 38.4 GB free. You have 214 GB.`                   | `You have 214 GB.`                                      | C4 answers the free space; the _needed_ half is a byte total of a download plan, which nothing carries                             | #206  |
| `11,798 files already match on both sides`               | row omitted                                             | a count of files the plan does **not** act on, absent from `PlanSummary` by construction                                           | #242  |
| `worked out 40 seconds ago · about 25 minutes to finish` | the first clause                                        | `run_dry_run` reports what would happen, never how long it would take                                                              | #229  |
| `159 of 471 done · about 17 minutes left`                | the first clause                                        | same estimate                                                                                                                      | #229  |
| `nothing deleted · 2 conflicts kept as copies`           | drawn from the approved plan; omitted with none in hand | no reply carries a per-pass summary _while the pass runs_                                                                          | #213  |
| `12,480 files, 41.2 GB.`                                 | dropped from the consent sub-line                       | no command reports index-wide totals                                                                                               | #207  |
| the install command box                                  | omitted                                                 | see 79d                                                                                                                            | #218  |

**`Nothing will be deleted · on either side` is conditional**, and that is not an omission. It is the
row `09-onboarding.md` calls the whole point of the step — "zero destructive actions stated as an
explicit positive fact, not a counter reading 0" — so it is drawn only when the plan really has
none. A plan that would delete something cannot say it, and no frame draws that state.

### 79d. The command box, and three other buttons with no destination

§72 recorded that `sudo apt install proton-drive` works on no distribution, by this project's own
documentation, and told S7 not to ship a copyable command box from that table until the design
settles what the real instruction is. It has not, and this repo's own README says only "`proton-drive`
must be installed, authenticated, and on your `PATH` first" — there is no true command to put in the
box. So the box goes and the dialog is `mark · title · body · Check again`. The `Detected Debian`
half of C5 is what still works, and it stays.

Three more drawn buttons had nowhere to go, all for the same structural reason — the takeover covers
everything and cannot be dismissed, so there was no sub-screen to visit and come back from (#244).
**Two of the three are drawn now, and open a sub-screen INSIDE the takeover** (#244):

- **`Add skip rules`** — opens onboarding's own rules editor: the staged list, a `Remove` per rule,
  an add field, and a `Back` that closes it. The rules are staged rather than written, and go into
  the config with the folder pair when `See what will happen` writes it — so the rehearsal on step 2
  is a rehearsal of them. The editor is still `8a Skip rules` as well, and the panel keeps its
  sentence: "or any time later in Settings" is now the second way in rather than the only one.
- **`See all 471 actions`** — opens the plan itself, row by row, with the Plan screen's own row
  grammar (`markOf`, `pathOf`, `outcomeOf`) imported rather than rewritten. Its heading counts what
  the button counts (`actionsThatHappen`), and it therefore leaves the `skip_unsupported` rows out
  of the list as well as out of the number. A windowed reply names what it is not showing.
- **`Installation help` is still not drawn, and #231 is not what was holding it.** `open_remote`
  proves the command surface can open a URL — that half landed. What is missing is a URL worth
  opening: this project's own documentation records that no distribution packages the CLI and tells
  the reader to follow "its own documentation" without naming where that is (#218). A button that
  opened something plausible would be the drawn command box's own bug one layer up.

**A detour is orthogonal to the step, which is what makes `Back` correct rather than remembered.**
The step does not move while a sub-screen is open, so closing one restores nothing — there is
nothing to restore. The header still reads `step 1 of 2` inside a detour for the same reason.

A disabled button would be worse than an absent one (§76's own rule, and `button()` attaches no
listener to a disabled kind — so one armed later paints live and does nothing).

### 79e. The remote path is a field where the frame draws a line

`9a Folders` draws `Browse Proton Drive…` under the remote card. S6 already settled that a remote
folder cannot be browsed for — nothing browses Proton Drive, and the daemon's `list` verb (#99) has
no picker in front of it — and drew
`8a Settings`' remote side as a field with no button for exactly that reason. S7 takes the same
answer: the button goes, and the path itself becomes editable so the step can still produce a pair.

**That node is deliberately unmapped.** A UA rule pins an `<input>`'s `overflow` to `clip` and its
`display` to `inline-block`, and the frame draws a `<div>`; the two can never agree on either. That
is a construction difference, not a missing capability, so it does not belong in
`known-deviations.mjs` — whose own bar says a node that was never mapped is not what that file is
for. The local path stays a `<div>` and is mapped.

The asymmetry has one visible consequence: a path longer than the card wraps on the local side
(`word-break: break-all`, as drawn) and scrolls on the remote one. Measured at 1042×766 with a
300-character path — the window still fits, nothing paints over the footer.

### 79f. Two undrawn bodies, because a rehearsal takes time and can fail

Step 2 has to run `run_dry_run` before it can say anything, and the flow's own step 1 writes the
config that command reads. Neither the waiting state nor the failure is drawn, and without them a
machine with no `proton-syncd` on its `PATH` gets a blank middle. Both borrow shapes that exist:
`5a Checking`'s dry-run mark and copy for the wait, S4's failed body for the failure, with the
daemon's string quoted verbatim (voice rule 4).

The error block caps its height and wraps anywhere, which `.pl-failed-error` did not until §96 gave
it the same bound by a different spelling: a daemon failing with a long stderr would otherwise grow
the block until it painted over the footer. Driven with a 10 KB error at 1042×766 — 766, no overlap.

### 79g. Four primitives, corrected by the screen that drew them first

Same pattern as §78h: F1–F6 wrote these against frames nothing had rendered yet.

- **The step chip is not a pill.** `CHIP.step` existed with `border: null, dot: null`, but `.chip`
  carries `padding:5px 12px`, `border-radius:99px`, `display:flex` and a 7px gap — and both `9a`
  window frames draw `step N of 2` as bare mono text. It takes its own class rather than four
  resets, and `updateHeader` now finds the chip by `[data-variant]`: a `.chip` selector returns null
  on onboarding and threw on every patch pass.
- **The checkbox's wrapper painted the box.** `.checkbox` set `font-size:13px` and
  `color:--text-bright`, which the 17px box inherited; `9a Consent` draws box and sentence as
  siblings with only the sentence carrying that pair. Both moved to `.checkbox-label`, and the
  wrapper gained `position:relative` for the visually-hidden input beside the box.

  **The box's own `position:relative` was then removed, which was wrong, and the correction is the
  interesting half.** The box is the containing block of `::after` — the tick — so without it a
  ticked box draws its checkmark against the wrapper instead, 1px left and 2px up of where F5's
  offsets were written. Nothing could see it: `9a Consent` is drawn UNCHECKED, the fixture pins
  `agreed: false`, and the tick is the only thing in the app that state contains. But the frame does
  say `static`, and `position` is asserted. So it lives on the CHECKED rule, which is exact in the
  drawn state and correct in the one no frame draws. Found by review; the two states are measured
  (`static`/no tick, then `relative` with the tick at `left:5px top:1px` inside the box).

- **`.dot` had no directional fill.** `9a Folders`' two 8px side markers are `--up-label` /
  `--down-label`; the tone table had inert, destructive and decision only.
- **`bytes()` wrote a trailing `.0`.** `214.0 GB` where `9a Review` draws `214 GB`, and `500.0 GB`
  where `9a Folders` draws `500 GB`. Its own comment already said KB stays whole; a whole number of
  anything does. No frame draws an `X.0` size, so nothing else moved.

### 79h. The gate

67,109 assertions / 0 failures, 275/275 drawn strings, 240 tests. 36 of 51 frames mapped. Fifteen
Phase-1 deviations recorded against #240, #241, #242, #243, #244, #191, #206, #207, #213, #218 and
#229; five gaps filed as #240–#244.

### 79i. Three transitions the frames cannot show, found by reading the daemon rather than the drawing

**A completed pass is not a successful one.** `advanceOnboardingStage` opened the consent dialog on
`reconcile_seq` advancing — and `reconcile_blocking` bumps that counter on failure too ("the attempt
is complete (recorded either way)", `src/daemon.rs`), recording the reason in `last_error`. So a
first sync that failed — the CLI not logged in, the boot-PATH class this project has already shipped
once — opened a dialog saying `Both sides now match` and `Nothing was deleted` over a merge that did
nothing. The decision moved into `mergeOutcomeOf`, pure and tested, because every arm of it is a
claim about someone's files; a failure now goes back to step 2 with the daemon's own string.

**The latch re-entered mid-merge.** `derive_state` returns `firstRun` for a reachable daemon that has
never synced, which is exactly what the daemon is between `start_service` and its first pass
beginning — so the takeover would reopen behind the merge dialog, drawing step 2 and its stale plan.
`nextOnboardingLatch` stays pure (it is PR #131's routing and is carried forward verbatim); the
caller overrides it while the flow is past the takeover, which is a thing the daemon state cannot
say.

**Nothing reset the flow.** `onboardingStep`, the plan and the ticked consent box all survived the
run. A later re-entry would have opened at step 2 on yesterday's rehearsal with the box already
ticked — so consent completing resets the lot.

`resume` was left as it was, deliberately: it resolves with its error inside the payload rather than
rejecting, so the dialog closes on the round trip landing rather than on the daemon being resumed.
That is the same call `5a Plan`'s `Run this sync` makes (§76) — the main screen behind is where both
outcomes are legible, and holding someone inside a consent they have already given is worse.

### 79j. `append(null)` printed the word

Copilot's two findings on the PR, both real and both validated against the code before being taken.

The merge dialog's footer omits its sentence when there is no plan in hand (79c) — and omitted it by
passing `null` to `Element.append`, which **stringifies its argument**. So `9a First sync` rendered
the literal text `null` beside `Pause`, on the one dialog nobody had looked at with a plan absent.
Every gate was green over it: the style gate compares STAMPED nodes and a stray text node is not one,
the copy gate reads the deck rather than the DOM, and the deviation row for the spacer beside it had
pinned the width the null text produced. `app.js`'s own note on `replaceChildren(null)` records this
exact bug from the F4 rewrite — "it took looking at a screenshot" — so this is the second time, and
the children are filtered now.

The second: `FIDELITY_SHOW` (a new env override for the harness's 40-failure print cap) took
`Number(...)` without checking it, and `Number("")` is `0` while `Number("x")` is `NaN` — either
prints nothing while the summary line still says how many failed. It falls back to 40 unless the
value is a positive finite number.

### 79k. What an adversarial review found, and the shape it was

Three reviewers over the branch produced 21 findings; a refutation pass and hand-verification against
the daemon's own source settled them. Every one was in an undrawn state, a transition, or a
live-daemon shape — the fourth screen running where that is true of all of them.

**Two were the same bug seen twice, and both are about a first sync that fails.**
`self.last_sync` is set inside the Ok path of `reconcile_blocking_inner`, so a pass that fails never
sets it — and `mergeOutcomeOf`'s no-counter arm tested exactly that timestamp. A failed first sync on
a machine that had never synced therefore reported `waiting` forever, behind a dialog with no ✕ and
no Esc. The counter is what says a pass ran, on both arms. And `failOnboardingMerge` wrote its reason
into state nothing could render: the latch declines to re-enter once a pair is written, which is by
then always. The failure is latched, and holds the takeover open **only while the daemon is
unreachable** — a reachable daemon that failed a pass is the main screen's business, which is
`routes.js`'s own rule about not trapping someone in a wizard that cannot fix their problem.

**One was a trap I had built.** `Pause` in the merge dialog paused a daemon that then completes no
pass, so the flow waited forever behind a dialog nobody can dismiss. Pausing now ends the flow and
hands off to the main screen, which draws `Paused` and a `Resume`. The consent is not obtained on
that path; the daemon's delete guard is on by default, so every deletion still goes through the
Deletions screen.

**One was a trap S1 had already documented.** The merge mark's numeral was `pending_changes` — the
local filesystem-watch queue, which a pass driven by Proton carries empty. `screens/main.js` records
this in full ("the headline read `Syncing 0 changes` with a literal `0` inside the mark"), and the
first merge is by definition the pass with the emptiest queue. It is `action_total - action_index`
now: the files still to move, which is what the frame draws.

**One was a regression from a fix.** Holding the CLI dialog up across a re-check (`cliChecking ||`)
also held it up across the FIRST check, so every first run flashed "the command line tool isn't
installed" before anything had been checked. The re-check needs no flag: `cliPresence` still holds
the answer it is re-asking.

**One was the poll destroying an animation**, which is this project's oldest UI failure mode: the
merge dialog's signature carried a per-action counter, so the layer replaced its children twice a
second and restarted both of the hexagon's travelling segments. The signature is the shape now and
the two numbers are patched in place.

Also fixed: a folder pair the machine already has now beats the proposal (the latch enters on
`firstRun` too, and that daemon HAS a config — proposing `~/ProtonDrive` over it and writing it back
would repoint someone's daemon at a folder they never chose); free space is re-asked when the folder
changes rather than once per run; `differ(1)` said `1 files differ on both sides`; and focus lands on
the main screen's own action when the consent dialog closes.

**The consent's promise is enforced rather than asserted.** `pause` resolves with its error inside
the payload rather than rejecting — the shape §77 records for the status commands — so the single
fire-and-forget `command(api.pause)` that opened the consent dialog could silently not land, leaving
`Syncing stays paused until you agree.` beside a checkbox on a daemon that was still syncing. The
poll now re-asks until the daemon reports itself paused, capped at five attempts.

**One finding was handed on rather than fixed.** `derive_state` falls through to `Idle` for a daemon
whose last pass failed for any non-auth reason, so the main screen says `Everything is up to date`
over it — including on the hand-off S7 makes when a first sync fails against a reachable daemon. It
is the derivation, not any one screen, and nothing in the GUI reads `last_error` at all. Filed as
**#246**.

**One thing is recorded rather than fixed.** `Nothing gets deleted today` is drawn unconditionally
while the fact row below it is gated on `destructive_actions`. The planner's bootstrap arm emits no
`Delete` or `Purge` at all (`plan_bootstrap_entity_action`), so the case needs a pre-existing `.sync`
index under the chosen local root — re-running setup over an old sync root. The deck has no headline
for that situation and inventing one is a design decision, not a build one; the enumerated claim is
gated, and this is the question to put to the design.

## The fidelity harness, between S7 and S8

## 80. The twelve unstamped slots were eight false alarms, four real, and no gate

S5 gave `assert.mjs` a report: per frame, the fid slots a fixture declares that the running app
never stamped. It exists because the style gate compares STAMPED nodes, so a block that renders
nothing stamps nothing and is simply not compared — `7a Never synced` rendered an empty body through
four separate causes with every gate green, and it was found by dumping attributes by hand.

The report named twelve slots across seven frames — one printed line per frame, slots joined on it —
every run, without anyone having to do anything about them. **Sorted against the frames themselves,
they were two entirely different things:**

| Slots                                                            | The frame draws the node? | Verdict                      |
| ---------------------------------------------------------------- | ------------------------- | ---------------------------- |
| `meta` / `action` on five compact frames                         | **no**                    | inert — not a finding        |
| `transferTrack` / `transferFill` on `2a Syncing`, `2a Needs you` | **yes**                   | a block that renders nothing |

**The eight were noise, and worse than noise.** `compactFids` is a factory over four tree shapes and
hands every frame in a shape the whole slot vocabulary, so `10a Settled` declaring `meta` says the
SHAPE has a meta line, not that this panel draws one. Each of the eight resolves to a key that
exists in no node of the frame declaring it. `check-fixtures.mjs` already tolerates precisely this —
its rule is "alive somewhere", not "alive here", and its header records that requiring per-frame
resolution flagged 23 of 180 keys with all 23 legitimate. So the report was contradicting the gate
next door, and giving itself a permanent floor of eight benign lines for a real ninth to hide in.

`assert.mjs` now filters on the frame's own nodes before reporting, which removes all eight
statically and permanently, and would report a compact `meta` again the day the prototype grows one.

**The four are a Phase-1 omission that had never been written down.** Both frames draw a 460×2 track
under the in-flight transfer with the fill at 64% and 82%. `main.js` computes it correctly —
`bytes_done / bytes_total` — and gets `null` every time, because `TransferActivity` carries
`bytes_total` on an upload and `bytes_done` on a download and never both (§63, #98). `transferRow`
draws no track rather than a bar at 0%, which would read as stalled, or at an invented fraction,
which would be worse. Correct behaviour, and invisible to every gate.

**They cannot be `KNOWN_DEVIATIONS` rows.** That list absorbs a failing assertion by name, and an
unstamped node produces no assertion at all — a row for one would never fire, and the rule that
keeps that list honest (an entry that stops failing fails the build) would reject it on sight. So
`KNOWN_UNSTAMPED` is a second list, with the same rule transposed: a row that is no longer observed
fails the build, whether the capability landed, the prototype moved the node, or the frame stopped
being mapped. Each row pins the node key, which is what tells the second case apart from the first.

**And an unstamped, drawn, unrecorded slot is now a failure rather than a line of output.** It also
runs BEFORE the unmapped-frame bail-out, which is where the gate had its own failure inverted:
blanking half a screen left enough stamps to keep the frame in the mapped set and the missing half
was a finding, while blanking ALL of it dropped the frame into the "screen not built" printout and
the run stayed green. Measured — making `7a Never synced` stamp nothing gave `35/51 frames mapped,
66,362 assertions, 0 failures`, exit 0, with 806 assertions gone and the frame's name folded into a
truncated list. That frame is the one this mechanism exists for. (Those are this build's numbers, and
they are a record rather than a standing measurement: §81 stamps five more of that frame's nodes, so
the same blanking costs 1,101 assertions after it.)

**It did NOT catch the case it was built for, and that was worth stating plainly.** §81 closed that
half; the rest of this section records the shortfall as it stood, because the way it was found is the
point. The intended motive was S8: the tray panel reuses the compact panel, whose two progress bars
draw today only because the fixture hands them `progress: 0.64` and `0.31` as literals — there is no
live caller yet. The moment S8 wires it to `SyncActivity`, #98 removes the fraction and #211 gives
the second row a real source. (Both halves have since happened, and only one of them the way this
predicted: S8 wired the panel to `SyncActivity`, and #211 landed the queue — so the second row is
real but it is the queue's next transfer, not a second one in flight. §63c.)

**The gate read static fid slots only** — 620 of 838, because a factory slot resolves to a different
key per call — and the compact panel declares `transferTrack`/`transferFill` as factories. So that
exact regression passed: setting `progress: null` on both `10a Syncing` transfers gave `36/51 frames
mapped, 66,936 assertions, 0 failures`, exit 0, with 232 assertions gone and no unstamped output at
all. Only a total blank of that panel was caught, through its static `headline`/`sub`/`hero`.

Probing factories the way `check-fixtures.mjs` does is tractable, and was measured rather than
waved away: 33 further findings, 19 of them at index 0 alone, across `7a File lookup`,
`7a Never synced`, `9a Review` and five more. Each needed the same sorting the twelve got, so it was
filed as **#248** rather than bolted onto this one. The claim made here was the narrower true one:
the static half is complete and binding, and the half that would have caught S8 is filed with its
measurement attached.

## 81. The other 218 slots: 23 nodes nobody was comparing, and 15 nobody had written down

§80 shipped the unstamped gate over static fid slots and said out loud that it did not catch the
case it was built for. This is that half (#248), and the split it produced is the reason it was
worth doing separately: of 39 drawn-but-unstamped slots the factory probe found, **23 were nodes the
app draws and simply never stamped, 15 were real Phase-1 omissions with nothing on file, and one
was neither.**

### What the probe is

A factory slot (`row: (i) => …`) resolves to a different key per call, so `probeSlot` calls it over a
10³ index grid and keeps the keys that frame draws. That is stricter than `check-fixtures.mjs`'s
`value(i, 0, 0)`, on purpose: `sideRowNote(s, i)` is keyed by side **and** row, so a single axis
reaches only row 0 of each side. The grid finds 39 where one axis finds 33, and all six extra are
further rows of clusters the single axis already found — it completes findings rather than widening
them. Two limits remain, both failing safe because the key is simply never produced: an index past
`PROBE_DEPTH`, and a factory wanting a non-numeric argument. Neither bites today, and both were
checked rather than assumed — raising the depth from 10 to 30 reaches not one drawn key that 10 does
not, and every fid factory takes at most three arguments and is keyed by position.

### The 23 that were a mapping gap

Not missing capabilities — nodes the app renders correctly and the gate was blind to, so 1,390
assertions were not being made:

| Frame                                                         | Slots                                                                 | What was unstamped                                                                                                               |
| ------------------------------------------------------------- | --------------------------------------------------------------------- | -------------------------------------------------------------------------------------------------------------------------------- |
| `3a Conflicts cleared`, `7a Activity quiet`, `7a File lookup` | `hexPath` ×2 each                                                     | the hexagon's two paths — S1 and S3 stamp theirs, these three stamped only the `<svg>`, leaving the ring and the tick uncompared |
| `7a File lookup`                                              | `card`, `cardLabel`, `cardBox`, `cardMeta`, `cardSize`, `cardPath` ×2 | `lookupCard` built the whole two-card block with no `fid` call in it at all                                                      |
| `7a Never synced`                                             | `rulePattern`, `ruleRowPath` ×2, `ruleRowNote` ×2                     | the rule's pattern span, and both children of each sample row (the row itself was stamped)                                       |

Stamping them found two real CSS differences that no gate could previously see:

- **`.activity-card-meta`** set `font-family`/`font-size`/`color` on the flex row; the frame records
  the row inheriting the body's sans 16px and each **span** setting mono 11.5px itself. Identical
  pixels — the row has no text of its own — but seven assertions on a card that looks exactly right.
- **`.path-note`** declared `flex: none`; the frame records the default `flex-shrink: 1`. `.path-name`
  beside it is `flex:1; min-width:0` and truncates, so the note is never the thing asked to give way.

### The 15 that were real, and the one that was neither

Fifteen became `KNOWN_UNSTAMPED` rows, each pinned to the node it explains:

| Frame          | Slots                                      | Why Phase 1 draws nothing                                                                                                                                      | Issue      |
| -------------- | ------------------------------------------ | -------------------------------------------------------------------------------------------------------------------------------------------------------------- | ---------- |
| `4a Deletions` | `cardFacts`, `cardFact` ×3                 | the folder card's strip is absent entirely (`factsOf` builds no fact for a directory); the file card's first span is `deleted here 6m ago`, a re-stamped field | #208, #225 |
| `5a Plan safe` | `sideRowNote` ×5                           | a size beside every file the rehearsal will move, and the report carries no per-file size                                                                      | #191       |
| `9a Folders`   | `cardButton`, `sideNote`                   | `Browse Proton Drive…` has nowhere to go, and no command sees the account or its quota                                                                         | #99, #241  |
| `9a Review`    | `fact`, `factDot`, `factLabel`, `factNote` | `11,798 files already match on both sides` — the summary counts what the plan will DO                                                                          | #242       |

The last of the sixteen was neither a gap nor an omission. `9a Folders`' remote `cardPath` **is** drawn — as an
`<input>`, because the remote root is the editable one — and §79 records why an `<input>` and a drawn
`<div>` can never agree on `display` or `overflow`. Reporting it as "a block that renders nothing"
would be false, and recording an excuse for it would put a construction difference in a list reserved
for missing capabilities. So the fixture stops declaring the slot: `cardPath` returns `null` at
`s === 1`, the way `rulePattern` returns `null` off-index. **A factory's `null` is how a mapping says
"no node here".**

### One row is one node

`KNOWN_UNSTAMPED` identity became `frame` + `slot` + `key`. It reads like bookkeeping and it is what
keeps the list honest: a factory slot covers a run of siblings, so one `(frame, slot)` can be
unstamped at four keys for four different reasons — `9a Review`'s `fact` row 0 is #242 and its row 3
would be something else. Matching on `frame|slot` would let the first row's reason silently vouch for
all of them. It also means a node that moved fails twice — the row goes stale, the new key arrives
unexplained — instead of being absorbed by a mapping nobody re-checked.

### The one suppression with no rule of its own

Review found the weak point, and it is worth stating rather than leaving to be discovered. Every
suppression in this subsystem self-invalidates: `KNOWN_DEVIATIONS` fails when an entry stops failing,
`KNOWN_UNSTAMPED` fails when a row stops being observed, `SETTLED_FRAMES` is cross-checked against
`index.json` — and that gate's own comment states the rule, _"a hardcoded label list that nothing
cross-checks is a gate that can switch itself off"_.

Leaving a node **undeclared** has no such rule. `cardPath` returning `null` at `s === 1` is the only
honest answer available for that node, and nothing targets the null itself. The comparison to
`rulePattern`'s null does not hold and the first version of this section made it anyway: `rulePattern`
nulls at indices where **its own node does not exist** (one rule, so no `rulePattern(1)`), while
`cardPath(1)`'s node is drawn.

The first version of this section then went one claim too far — it said no gate would notice when the
suppression expired. **Two do, and review measured both.** Stamp the slot and `fid()` null-checks the
factory rather than its result, so it writes `data-fid="9a Folders:null"`; `assert.mjs` finds no such
key and reports a `(mapping)` failure, exit 1. Let #99 land and replace the `<input>` with an
unstamped `<div>` and the **parent** card's exact-pixel `box.h "147 vs 58"` deviation goes stale _and_
fails, exit 1 — an exact-detail pin on a parent covers a child's construction changing under it.

**Both covers are incidental, and that is the part worth keeping.** Nothing decided this node would
be caught; it is caught because a neighbour happens to be pinned to the pixel. No rule says which
undeclared nodes are deliberate, so the next one may have no such neighbour. That is the convention's
blind spot rather than this slot's, and it is bigger than this section:
**268 of the 1,452 drawn nodes on mapped frames are claimed by no slot at all — 18%.** Most are
correct (S4 leaves `5a Checking`'s progress line undeclared, S5 the four history rows, S6 the
twenty-bar chart), and almost none of those reasons is written where a gate can read it. Sorting 268
nodes is the same unit of work the twelve and the thirty-nine were, so it is **#250** with the
measurement attached rather than a clause bolted on here.

### The check that says it worked

The motivating scenario of §80, run against this build: `progress: null` on both `10a Syncing`
transfers now **exits 1**, naming `transferTrack` and `transferFill` at all four nodes. That claim
was the one #247 shipped untested, because it was the argument for doing the work.

---

## The system tray (S8)

## 82. The tray protocol could not deliver the tray, and five other things the frames do not say

`10-tray.md`'s whole interaction is _left click opens the compact panel_. `IMPLEMENTATION-PLAN.md` §6
flagged the rendering half of that as the architectural risk ("libayatana-appindicator cannot render
a hexagon and a seam") and told S8 to prototype before estimating. The prototype found something
worse than a rendering limit: **there is no click**.

`tray-icon-0.24.1/src/platform_impl/gtk/` contains no reference to `TrayIconEvent`. Not a disabled
path, not a feature flag — no code that emits one. Introspecting the item the shipped app was
publishing confirms it from the other side: it exposes `Scroll` and `SecondaryActivate` and **no
`Activate` and no `ContextMenu`**, so a host has nothing to call even if it wanted to. Two paths in
`tray.rs` were therefore dead on Linux and read as working code: the `Click` handler ("left click
toggles the window into view", which never happened), and every `set_tooltip` — literally `Ok(())`
with the argument dropped, so a status string was built every five seconds and discarded.

The resolution is to speak `org.kde.StatusNotifierItem` directly, modelled on **xembedsniproxy** —
KDE's own X11 bridge, whose published item is `Activate` + `ContextMenu` with `ItemIsMenu = false`
and no `Menu` object at all. Copying a shape Plasma itself ships is better evidence about what a host
honours than any paragraph of the spec. Measured on a live session before a line was written: the
item registered, was read, and a left click arrived as `Activate(3192, 2112)` — with the click's
screen coordinates, which also disposes of §6's second sub-risk (the indicator's position is not
queryable: `rect()` returns `None`).

**a. `--hex-glyph-fg` is a new token, and the tray glyph is the first thing to need it.** F2 wrote
the `family: "tray"` branch of `renderHexagon` in anticipation and nothing had ever passed the flag.
Drawn for the first time, settled and paused stroked `--hex-settled-track` / `--hex-paused-track` —
the dark ring the in-window mark draws a check or a pair of bars _inside_. At 20px the outline IS the
mark, so the glyph came out a near-invisible grey on a dark panel. All ten marks on `10a Glyph
states` stroke at the foreground colour or at their tone; not one is a track. `10-tray.md` supplies
the light value itself ("the glyph inverts to `#14161A` with the same five forms").

**b. The sheet draws 20px, not 16.** The issue title and the doc's own table say 16; `10-tray.md`
gives a range ("rendered `15–20px`") and all ten marks on `10a Glyph states` measure exactly 20. The
range is what a desktop may scale the SVG to; 20 is what the design measured. The shipped SVG files
declare 16 — the bottom of the range — because that is a starting size for a scalable icon, not
geometry: the geometry is the `viewBox`.

**c. `mono` was a one-line flag and is now the property the set is built on.** It reached only the
syncing track, so the monochrome column drew needs-you in crimson and can't-reach in red — the exact
thing `10-tray.md` forbids ("state is carried by fill, not hue"). Disabling the fix now fails
precisely five nodes: the syncing segment, the needs-you path and circle, and both of can't-reach's
paths. Settled and paused are unaffected, because they have no hue to lose — which is the design's
own claim about why those two forms are shaped the way they are, confirmed by a gate.

**d. The syncing track becomes `currentColor` at 0.18 opacity.** A recoloured symbolic icon has
exactly one colour, so the track's `#3E454E` cannot survive as a literal: it would either clash with
whatever the panel chose or be recoloured to match the segment and vanish. 0.18 is the alpha that
reproduces `#3E454E` over the `#191C21` the sheet draws on, computed per channel (0.16 / 0.19 / 0.20
— the two colours are not the same hue) and averaged. The icon keeps the drawing's _ratio_ between
track and segment whatever colour the desktop picks.

**e. The tray's syncing glyph does not move.** `renderHexagon` puts `animation:hexup 2.4s linear
infinite` in a style attribute; there is no CSS engine behind a tray icon and the SNI protocol has no
notion of an animated one. The segment ships frozen where the sheet draws it, and the motion
`10-tray.md` asks for lives in the panel, which is a real webview. Animating by swapping icons on a
timer was considered and rejected: it is a D-Bus signal and a file write per frame, and the doc's own
rule is that the glyph updates "from the daemon's status stream, not a timer".

**f. The panel's state is S1's derivation, imported.** `screens/tray.js` calls `heroStateOf` from
`screens/main.js` rather than deciding again. The window and the tray are two views of one moment,
and someone who opens both and reads two different sentences has been told the app does not know what
is happening. Every rule in that function was paid for once — unreachable outranking everything,
`pending > 0` reading as syncing even when `syncing` is false, `authExpired` not falling through to a
false all-clear — and a second copy would relitigate all of them badly.

**g. Two states reach the tray that never reach the main screen, and no frame draws either.**
`app.js` intercepts `firstRun` with the onboarding takeover, so S1's derivation has no branch for it
at all and falls through to `settled` — `Up to date`, on a daemon that has never copied a file. The
tray has no takeover to hide behind. It draws the needs-you mark with no numeral (a `0` would present
an empty queue as a decision) and two sentences the deck does not have: `Nothing has synced yet` /
`Open Drive Sync to choose your two folders.` Written rather than measured, and kept close to what
exists — the v1 tray shipped `Nothing synced yet` as a disabled menu item.

`authExpired` keeps the struck mark it shares with `unreachable` (`11-notifications.md` puts an
outage and an expired session behind one icon) but **not its menu**. `Try again now` retries a sync,
which is not what an expired session needs, and `10-tray.md` asks exactly one thing of these rows —
that the label says what it does. So the panel is keyed by FORM and the menu by CAUSE, and
`deferToWindow` is a sixth _row set_ rather than a sixth _form_. There is no sixth form.

**h. `retrying in 40s · last reached 13:58` is omitted.** Nothing in the reply says when the next
attempt is, and an unreachable daemon is not answering to be asked. Omitted rather than filled, per
`14-behaviour-and-state.md`'s rule for a missing capability — the same call S1 makes on the settled
sub-line's file count (#207).

**i. The glyph updates on a 2s poll, not a stream.** `10-tray.md` asks for "the daemon's status
stream, not a timer". There is no stream: the control socket answers questions and does not push,
which is **#101 (E4)**, explicitly deferred. Two seconds matches the window's own cadence so the two
surfaces never disagree by more than one tick.

**j. Right-click opens the panel, not a native menu.** ~~`10-tray.md` gives right-click to the menu
alone by KDE convention. Delivering that needs `com.canonical.dbusmenu` — a second protocol with its
own layout-revision model, and an S8-sized task by itself. The panel contains every row that menu
would have, and a right click that produced nothing would read as a broken tray rather than as a
deliberate absence. Filed as a follow-up.~~ **RESOLVED by #252 — see §89.** The item publishes a
`Menu` object path and a `com.canonical.dbusmenu` service; `ContextMenu` stays implemented, because
it is what a host with no importer falls back to calling.

**k. The fallback menu folds the sub-labels into the label.** A session with no status-notifier host
gets the Tauri tray, because no indicator is worse than a text menu. A GTK menu item is a single
string, so `Close window` and `Quit` cannot carry the second baseline-aligned span the panel draws —
they carry `Close window — keeps syncing` and `Quit — stops syncing` instead. What they must not do
is lose the words, which is what the v1 build got right and for the same reason.

**l. `Review them` lands in the window, not on the deletions queue.** The two rows share a
destination. Routing `review` to a specific screen is a second question — the window may be mid-flow
— and the panel is dismissed by then either way.

**m. §45 is settled: `Quit` stops the daemon.** That section was left **Open** and assigned here.
S8 is the task that puts `Quit` and `stops syncing` on screen together, and after that there are two
options: the daemon stops, or the app prints a false label in the one place `10-tray.md` says it must
not ("the single worst misunderstanding a tray app can cause"). It stops, through the daemon's own
graceful `Shutdown`. `Close window · keeps syncing` sits directly above it and is unchanged. A
failure to reach the daemon is deliberately not fatal to the quit: a wedged daemon must not leave
someone unable to close the app.

### Four bugs only a real desktop could find

Every one of these compiles, renders, and passes every gate in the build.

1. **The click is in physical pixels.** `Activate(3192, 2112)` on a 3840×2160 display at scale 2 is
   not the 1920×1080 space `LogicalPosition` works in. The arithmetic was right and the units were
   not, so the panel opened in the middle of the screen.
2. **A window that has never been shown has no current monitor.** `place` returned early on that,
   silently, leaving the window manager's placement — dead centre, at exactly `(3840 − 724) / 2`. A
   centred popover is not a positioning bug; it is the absence of positioning.
3. **On X11 a position set before mapping is advisory.** KWin ignored it. The position is applied
   again after `show`, which is the call that lands.
4. **A blur only means something if the window ever had focus.** Focus-stealing prevention handed
   focus straight back, and the resulting `Focused(false)` was indistinguishable from the user
   clicking away — so the panel showed and hid in the same breath, looking exactly like a panel that
   never opened.

## Notifications (S9)

## 83. Two of the five `11a` frames are a Settings tab, and it is a tab the frames do not draw

`11-notifications.md` specifies a settings surface and gives it no home. `08-settings.md` enumerates
four pills — Folders · What to skip · Deletions · Advanced — and no frame draws a fifth, so this had
to be decided rather than read. Four pieces of evidence, all pointing one way:

- the prototype's own caption above the frame reads **"the settings tab — three choices, not twelve
  switches"**;
- `14-behaviour-and-state.md`'s fallback table names the section **Notifications** (_"Hide the
  section; default to the four events"_);
- the first card's body is `The four events on the left`, which is a claim about a layout with the
  rules sheet beside it;
- and `11a Settings` is drawn with the **identical chrome** `8a Deletions tab` puts around its crop
  — `#0A0B0D` on a 1px `#1A1D22`, radius 12, padding 24/26, `0 24px 60px rgba(0,0,0,.6)`. That frame
  is a known 600px re-render of a tab inside the 1040 window, and this one is the same object at 520.

So both are **crops** (`frame-classes.mjs` moved them out of the `notification` class F9 put them in,
which F9's own note says was a placeholder for "nothing had yet decided where they live"), and the
pill row gains a fifth tab. It sits **fourth**, before Advanced: Advanced is the technical drawer and
`08-settings.md` puts the dangerous things behind it, and a plain-language preference in front of it
is the product order.

**It costs the gate nothing.** The drawn pill row is 1040 wide and left-aligned, so a fifth pill
moves none of the four boxes `8a Settings` and `8a Skip rules` assert. What it did cost was the
mapping: `tab` was keyed by POSITION, so the new pill was compared against `Advanced` and reported a
width difference between two different words. `settingsFids`' `tab` is keyed by tab **id** now and
answers `undefined` for a tab no frame draws — and `fid()` learned to read that as "no node here"
rather than stamping `…:undefined`, which fails as "no such node key" and reads like a stale mapping.

`NOTIFY.settings.tab` is the one S9 string in `copy-gate.mjs`'s `NOT_DRAWN`, with that reason.

### 83a. The tab is 210px taller than the window, and no gate could have said so

`11a Rules` is 633px tall at the 600px it is drawn. The fit gate runs on `window`-class frames alone,
both halves of this tab are crops, and **nothing in the harness renders the tab at 1040×764** — so it
was measured by hand: the window came out **974px against 764**, with the reference sheet running
straight through the footer. `02-shell.md` calls that "a real bug found twice during this design".

The rules panel scrolls inside a wrapper, which is `.settings-rules-scroll`'s shape one screen over:
the panel's own box is what `11a Rules` describes, and an `overflow` declared on it would put a
property of the app's layout on the node the gate compares.

**The general form, for S10 and anything after it:** a crop is rendered inside a window and the fit
gate does not know that. Four frames are crops today and two of them (`8a Deletions tab`,
`8a Schedule monthly`) are short enough that it has never mattered.

## 84. `tauri-plugin-notification` cannot express either half of this design

The plugin was a dependency from v1 and `commands::notify` — title and body, no caller anywhere in
the frontend — was the whole of the notification surface. Two properties `11-notifications.md`
requires are unreachable through it, and both are in its source rather than in a changelog:

- **Action buttons.** `desktop.rs` builds a `notify_rust::Notification` from a title, a body and an
  icon and never calls `.action()`. `Keep them` / `Review`, `Compare` / `Later` and the outage pair
  are the spec; _"the absence of a Delete button is a deliberate safety property, not an omission"_
  (IMPLEMENTATION-PLAN §6) is the rest of it.
- **`replaces_id`.** "Never stack more than one Drive Sync banner" is that argument on the wire, and
  the plugin does not expose it.

So `src/notify.rs` speaks `org.freedesktop.Notifications` over the zbus connection S8 already added,
and the plugin (with its `notification:default` capability grant) is removed. The risk §6 flags —
_"the notification server may not support actions"_ — was **measured before any of it was written**:
`GetCapabilities` on this project's session answers `actions`, `persistence`, `inline-reply` and
more, against `GetServerInformation` `("Plasma", "KDE", "6.7.4", "1.2")`. A server that does not
advertise `actions` still gets the banner and is logged once, at connect; the sentence is the part
that matters and both actions are reachable in the window anyway.

### 84a. The banner we draw is not shown anywhere yet

`11-notifications.md`: _"Use the desktop's own notification chrome where the platform provides it —
these values are for when it doesn't."_ `ui/notification.js` builds both halves — `renderBanner`
draws the 372/520px banner the frames specify, `payloadFor` flattens the same spec for the wire — so
the sentence a desktop shows and the one we would draw come from one builder rather than two that
agree today.

**What is not built is the window to put it in.** A desktop with no notification server would need a
borderless, always-on-top webview positioned in a corner and dismissed on a timer — which is
`panel.rs` again, and S8 records what that took (physical pixels, a monitor a never-shown window
does not have, an X11 position that is advisory before mapping, a blur that fires before focus is
ever held). GNOME and KDE both ship a server, so this is the fallback for a bare window manager. A
failed send is logged, once, and the same event is still in the window and on the tray glyph.

Filed as a follow-up. The renderer is not dead code in the meantime: it is what the five `11a`
frames are asserted against.

**`replaces_id` is tracked, not assumed.** Measured on the same session rather than read off the
spec: `Notify` with `replaces_id: 0` returned `26`, a second `Notify` with `replaces_id: 26` returned
**`26`** — replaced in place, not stacked — and a `replaces_id` the server had never issued
(`999999`) came back as `999999`, so Plasma takes an arbitrary id rather than allocating around it.
The spec does not require any of that (it says only that the returned value equals `replaces_id`),
and a server that allocates a fresh id for one it has forgotten would stack the banner this rule
exists to prevent. So the live id is tracked: a `NotificationClosed` for ours clears it and the next
banner starts from `0`. `ActionInvoked` is
broadcast to every listener on the bus, so the id is matched against ours before anything runs;
without that, clicking `Discard` in another application's banner would fire one of ours.

## 85. The triggers are in the webview, and the copy is the reason

`10-tray.md`'s poll is in Rust and this is not, which is worth stating because the reverse looks
safer. The four triggers produce SENTENCES, and IMPLEMENTATION-PLAN §3.4 puts every sentence in one
module _"so a string can't drift between the screen, the tray and the notification"_ — `copy-gate.mjs`
checks that module against the frames, and a second copy in Rust would be outside every gate in the
build. `notifier.js` is pure and `ui/notification.js` builds the banner; `notify.rs` carries no
user-visible text at all, not even the app name.

The window is **hidden, not closed** (`lib.rs`'s `CloseRequested` arm calls `window.hide()`), so the
webview and its poll outlive the tray click. Its cadence drops from 2s to 10s when unfocused
(`scheduleNextPoll` keys on `document.hasFocus()`), which is well inside every threshold here — the
shortest is the 30-second coalescing window.

**One thing here is not verified and cannot be from a headless check: whether WebKitGTK throttles or
suspends a hidden window's timers.** The engine throttles a non-visible page and the documented
floor is 1s, which the 10s cadence already clears — but "throttled to 1s" and "suspended until shown"
are different answers and only a run on a real desktop with the window hidden distinguishes them. If
it turns out to be suspension, the fix is not a shorter interval: the triggers move to the Rust poll
in `tray.rs`, which runs regardless, and the copy has to move or be gated across the two languages
(§85 is the argument for why it is here). Worth a live check before Phase 1 closes.

### 85a. Two rules the design states and one it does not

- **Coalescing is a rate, not a delay.** _"Coalesce within a 30-second window"_ read as "hold the
  first event for 30 seconds" would hold the permanent-deletion warning for half a minute, and that
  banner's whole justification is that silence costs files. So the first is immediate, anything
  behind it inside the window **replaces** it rather than adding one, and the replacement is
  `replaces_id` — which is also "never stack more than one".
- **A more serious event jumps the window.** Waiting 25 seconds to say that files are about to be
  deleted, because a conflict banner went up five seconds ago, is the wrong way round. Severity is
  deletion > outage > conflict > first sync, and only a strictly higher one preempts.
- **The first-sync banner needs a witness.** `last_sync_epoch_secs` is set on _every_ successful pass
  and says nothing about which one was the first, so installing the GUI on a machine that has been
  syncing for a year would announce `Both sides now match` as though the wait had just ended. It
  fires only where this install actually watched the transition — and an unreachable daemon is not a
  witness that nothing has ever synced, because it is not an answer at all.

### 85b. The outage banner does not blame the session when the session is not the cause

`11a Outage`'s body is `Proton Drive is asking you to sign in again. 61 changes are waiting — nothing
is lost.` and the doc gives the trigger three causes: _"an outage, expired session, or full disk"_.
Saying the first about the third would be a false statement in the banner whose whole job is to be
believed, so the other two causes take the deck's own unreachable sentence
(`TRAY.unreachableBody`) — already drawn in `10a Offline`, already gated, and it opens with the
reassurance the same way.

**`Sign in` is not built and the button says `Try again now`.** §67a settled this for the main
screen's hero and nothing has changed: no command signs in, because the daemon reuses the
`proton-drive` CLI's keyring session.

**`Keep them` carries S3's caveat.** It sends what `Keep both files` sends, and #224 is unchanged —
`deny` revokes an approval and nothing on the wire refuses a withheld deletion durably. What it does
do is true: a withheld deletion is not applied, so the files stay.

## 86. `notify_policy` is GUI-local, and that is what makes "Never" safe

The daemon parses its config with `#[serde(deny_unknown_fields)]` on `FileConfig` _and_ its nested
tables, so one key it does not know makes it **fail to start**. `notify_policy` therefore lives in
the GUI's own `gui.toml` beside `proton-sync.toml`, never inside it.

That is not only a storage decision. `11-notifications.md`: _"'Never' must not change engine
behaviour — deletions still wait for approval. Turning off notifications is not consent."_ With the
key in a file the daemon never reads, the property holds by construction rather than by care.

A missing, unreadable, unparseable or unknown value reads back as `only_when_needed` — **fail loud,
not quiet**: the failure that matters is a broken file silently meaning `never`, which would drop the
one banner that can save someone's files. A _write_ refuses an unknown token instead, because a write
is a person choosing and storing something else would be the screen answering a question nobody
asked.

## 87. What the frames draw that Phase 1 cannot

| frame           | drawn                                         | Phase 1 draws                   | closed by |
| --------------- | --------------------------------------------- | ------------------------------- | --------- |
| `11a In situ` 1 | `1,204 photos would be deleted…`              | the queue's own length + noun   | G8 #208   |
| `11a In situ` 3 | `First sync finished — 12,480 files, 41.2 GB` | the sentence without the clause | G7 #207   |
| `11a Outage`    | `Sign in`                                     | `Try again now` (§67a)          | E6 #103   |

Eight `known-deviations.mjs` rows, four nodes per cause: each sentence wraps to one line where the
frame draws two, and the banner, its head, its text column and the sentence itself all measure it.

**The noun is not cosmetic.** `PendingDeletion` carries an `entity_kind`, so a queue that agrees on
one says `folder`/`folders` — a banner asking you to keep something it has misnamed is worse than
one that cannot count what is inside it.

## 88. One drawing slip, unmapped

`11a In situ`'s FIRST banner is the only one of the five that puts `letter-spacing:.01em` on the app
name; the other two in the same mock and both standalone banners draw `normal`. Four against one, at
0.12px. The component draws `normal` and that one node carries no slot — the call `8a Settings`'
`event_driven_reconcile` key line got, which is neither a mapped node nor a known-deviations row. The
spacer beside it goes with it: it is `flex:1`, so its width is the app name's subtracted from the row.

### 88a. What an adversarial review found, and the shape it was

Four lenses over the branch — the triggers, the Rust, the screen, the safety invariants and the
truth of these sections. Thirty findings, of which the ones worth recording are these, because none
of them is visible in a rendering and every gate in this repo was green with all of them present.

**Three change what a person sees.**

1. **A banner click was dropped about half the time.** A server emits `ActionInvoked` and
   `NotificationClosed` for the same press; both arrive in one socket read; `tokio::select!` picks
   between two ready branches at random. The close branch cleared the attribution the invoked branch
   needed, so `Keep them` did nothing, with no log line and nothing on screen. The id and event of
   the last banner now outlive its closing — only `replaces_id` is cleared.
2. **`First sync finished` announced itself on an established install.** The witness rule read a
   live `last_sync_epoch_secs == null` as "nothing has ever synced". `ControlShared::new` starts
   that field at `None`, so it is null after _every_ daemon restart — and Settings ships a
   `Restart it now` button. It counts only where nothing has ever been seen either.
3. **The delivered banner had no icon.** `icons.rs` writes the five symbolic SVGs to
   `$XDG_RUNTIME_DIR/proton-sync-tray` and hands that path to the STATUS-NOTIFIER HOST as
   `IconThemePath`. A notification server is a third process and is told nothing, so
   `proton-sync-attention-symbolic` resolved against the system icon themes, where this application
   installs nothing. The absolute path goes on the wire when the file is there.

**And the rest, each one line:** the signal listener exited silently on connection loss and left a
dead notifier behind, so nothing worked again for the life of the process · the send held the state
mutex across an untimed D-Bus call · `close()` forgot the id before the call that could fail · the
deletion body sent a user-chosen path unescaped to a server that advertises `body-markup` · the
`desktop-entry` hint named a file that does not exist · a staged policy outlived leaving the screen,
because `resetSettingsScreen` cleared ten fields and not that one · a `lastAt` in the future silenced
every banner until the clock caught up · a policy-only save offered to restart a daemon that has
never heard of the setting · a refused config save said `Nothing was saved` after the policy had
already been written · and the hard-rule test passed with both `throw`s deleted.

**The shape.** Eight of the ten are states that exist only in TIME — a restart, a signal ordering, a
connection that drops, a screen left and returned to. `11a In situ` is one rendering of one moment,
and this task's whole subject is when to speak; nothing that compares drawings can see any of it.
That is the same conclusion S1–S8 reached from the other side (§67b: "a frame is one rendering, and
this screen has twenty transitions"), arriving here as: **the fidelity harness cannot review a
trigger.** What reviewed these was reading the daemon's own source for what `last_sync` survives,
reading the freedesktop specification for what a server emits and in what order, and running
`decide` in sequence rather than once.

**Copilot took five passes over this branch, and its last three findings were all in code written to
answer the pass before** — the pattern this project has recorded twice already, and every one of
them arrived _suppressed_, which is the half of a Copilot review that has to be expanded by hand.

- **Pass 2.** The markup escaping added above defaulted to "the server does not parse markup" and
  only turned on when `GetCapabilities` said otherwise. That call is a round trip and can fail, so a
  transient bus error would have sent a user-chosen path into a parser that does. Unknown means
  escape now: an over-escaped `&amp;` in a path on a server we could not ask is not the same size of
  mistake as markup injection into a banner.
- **Pass 3.** `show` was given a deadline because it runs under the state mutex and a server that
  accepts a call and never answers holds that mutex for the life of the process — and
  `capabilities` runs on the same locked path, at connect, with no deadline at all. The same bug at
  the only other place it can happen, left behind by the commit that fixed the first one. `close`
  had it too; there is one `DBUS_DEADLINE` and all three use it.
- **Pass 3, and it does not hold.** `typeof null === "object"` does pass `loadNotifierState`'s
  guard, but object spread of `null` is a no-op by specification: `{ said: null }` resolves to
  `said: {}`, which is the empty state that function would have returned anyway, and nothing throws.
  Reproduced across six shapes before answering. Recorded as a comment on the guard rather than
  applied — a finding's premise is not its consequence.
- **Passes 4 and 5** generated nothing.

Two were caught by Copilot's first pass and are the same shape from a third direction: `payloadFor`
sent no `app`, which `NotifyPayload` requires with no default — so serde refused every payload and
no banner would ever have arrived — and a `#[cfg]` bound to one statement left a Linux-only type
managed unconditionally. Nothing checks one language's struct against the other's object; the test
does now.

## The tray's other half (#252)

## 89. The second protocol, and the one property that had to keep meaning what it meant

S8 delivered `10-tray.md` §Behaviour's first clause and deferred its second: left click opened the
compact panel, right click opened it too, because a native menu is `com.canonical.dbusmenu` — a
second protocol with its own layout and revision model — and shipping one protocol properly beat
shipping two half-way (§82j). This is the second clause.

**The premise was the first thing checked, because it was the thing that could break what worked.**
Publishing a `Menu` object path is what asks a host to import a menu, and a host that reads that
property could reasonably decide the item is menu-driven and stop sending `Activate` — which would
trade the panel for the menu rather than adding it. Plasma's own source answers it: the applet routes
a left click on `ItemIsMenu` **alone**,

```qml
// applets/systemtray/qml/StatusNotifierItem.qml
if (model.ItemIsMenu) { Plasmoid.openContextMenu(...) } else { Plasmoid.activate(...) }
```

and `statusnotifieritemsource.cpp` shows the same coin's other face — right click prefers the
importer when the item exposes a `Menu`, and falls back to calling `ContextMenu()` on the item when
it does not ("Could not find DBusMenu interface, falling back to calling `ContextMenu()`"). So
`ItemIsMenu` stays `false`, `ContextMenu` stays implemented rather than becoming dead code, and the
change is additive. Confirmed on the live item afterwards: with `Menu` published, `Activate` still
opened the panel.

**a. The layout was measured, not read off the specification.** Same method as §82: a libappindicator
item was published on this session and its own menu read back over the bus.

```text
GetLayout(0, -1, []) → (uint32 2, (0, {'children-display': <'submenu'>}, [
  <(2, {'label': <'Open Drive Sync'>}, @av [])>,
  <(3, {'enabled': <true>, 'type': <'separator'>}, @av [])>,
  <(4, {'enabled': <false>, 'label': <'Nothing synced yet'>}, @av [])>,
  <(5, {'label': <'Quit — stops syncing'>}, @av [])>]))
```

`Version = 3`, `Status = "normal"`, `TextDirection = "ltr"`, and every property a row does not need
is simply absent — `enabled` and `visible` default to true. That is the shape this ships, one flat
level under a root that declares `children-display: submenu`. Drop that one key and a host draws an
empty menu with the rows still on the wire.

**b. The numeric id is the ACTION, not the position, and that is a correctness fix rather than a
style.** dbusmenu names a row with an `i32`, and a host draws the layout it holds until it is told
otherwise — while these rows change with the daemon's state, on a two-second poll. So with
positional ids a click arrives as whatever now stands where the pointer was, and the worst of those
collisions is the pair `10-tray.md` cares most about: **`Close window — keeps syncing` and `Quit —
stops syncing` occupy the same position in different states.** `Close window` is 5th in the settled
set and 4th while syncing; `Quit` is 5th while syncing and 4th while paused. A menu opened on an
idle daemon and clicked once a pass had started would have stopped the daemon — on the row whose
entire job is to promise that it will not — and settled→syncing happens on every pass.

The id travels with the row instead, so a stale menu performs what its label promised or nothing at
all, and `Event` resolves against every row set rather than the one currently published.

_The first draft of this section named a different pair — a stale `Pause syncing` click landing on
`Quit` — and it was wrong: the paused set's third row is the separator, so that click would have hit
nothing. The claim is now a test that walks all five sets and computes the collisions
(`positions_collide_and_the_worst_pair_is_the_one_10_tray_md_names`), and it fails with an
instruction to fix the prose if the pair it names ever stops colliding. A rationale nobody can check
is a rationale that rots._

**c. `AboutToShow` always answers `true`.** The return means _you need to refresh before drawing_.
libdbusmenuqt honours it by re-reading the layout; answering `false` draws the cached copy, which for
a menu whose rows are a function of daemon state is how `Pause syncing` appears on a paused daemon.
It costs one round trip per right click. `LayoutUpdated` is emitted on the poll as well, for a host
that caches harder than the contract promises — Plasma was observed re-reading the layout within a
millisecond of each signal.

**d. `ItemsPropertiesUpdated` and `ItemActivationRequested` are not declared.** This replaces the
whole layout rather than patching properties, and nothing here asks a host to open the menu on the
program's behalf. A signal is matched by name on the bus, not found by introspection, so declaring
one that is never emitted buys nothing and reads as a capability.

**e. One table now, for three menus — and the third is in another language, so it is compared rather
than shared.** `#252` asked that the dbusmenu layout not become a third copy of rows that already
exist in `ui/compact.js`'s `TRAY_MENU` and again in `tray.rs`'s fallback. Two of the three are now
one `tray_menu.rs`: five row sets keyed by state, the ids `commands::tray_row` already dispatches,
and the em-dash fold of §82k in one place. The doc comment in `commands.rs` promising a
`FALLBACK_IDS` table "below" — a table that was never written — describes what exists now.

The panel's is a JS literal and cannot be the same object, which for one draft of this section made
"one table for three menus" an overstatement: nothing compared row MEMBERSHIP or ORDER across the
language boundary, only the seven label strings, so adding a row to `TRAY_MENU.paused` alone would
have given a left click and a right click two different menus for one daemon with every gate in both
languages green. `the_panel_and_the_native_menus_draw_the_same_rows_in_the_same_order` parses
`TRAY_MENU` and compares all six of its sets, separators included, against `rows_for` — including
`needsYou`, which has no Rust counterpart because S1's derivation folds it into the idle state, and
whose being _identical to settled_ is itself the invariant §82g rests on.

**f. The fallback menu never followed the daemon.** Found while giving it the shared table:
`install_fallback` returned early whenever the tray already existed, so its rows were whatever the
FIRST poll decided and stayed that way for the life of the process. A session started while the
daemon was down offered `Try again now` for ever and never `Pause syncing` — on the one desktop that
has no panel to correct it. The SNI item never had the bug, because it has always been re-fed on
every tick; the text menu is the copy that went stale quietly. It is rebuilt per state now.

**g. Nothing in Rust was comparing these labels to the copy deck.** `copy-gate.mjs` reads
`ui/copy.js` against the frames, and the frames are the panel; the native menus are a second copy of
the same words in a language that gate cannot read. `the_labels_are_the_copy_deck_s` parses the
`TRAY` block out of `ui/copy.js` and compares — including composing the folded label from the parsed
parts, so the fold itself is not written down twice.

**h. Four defects an adversarial review found — two of them S8's, two of them this branch's.** Every
gate was green with all four present, which is the fifth time this project has recorded that
sentence. (The first draft of this line said "three of them S8's" and contradicted its own list one
paragraph later: `EventGroup` was written here, and the fourth is a _regression_ — S8's behaviour was
right until a `Menu` path took the click away from it. A count is a claim like any other.)

1. **(this branch) `EventGroup`'s out-arg is `idErrors` — the ids that could NOT be handled** — and this returned
   every id it was sent, with a doc comment stating the inverted meaning as its justification. A host
   that batches (libdbusmenu-glib's client prefers `EventGroup` for a server advertising `Version >=
3`, which this does) would have been told every working click failed. The name is in the XML
   libdbusmenu-glib compiles in, read on this machine rather than from the specification:
   `<arg type="ai" name="idErrors" direction="out">`.
2. **(S8) `live` gated the state, not just the signals** (`sni.rs`). While a status-notifier host was
   away — plasmashell restarts on any panel-settings change — `update` returned early without
   writing the icon, the title or the rows, while the poll recorded the tick as shown. The host came
   back, read the item on registration, and got the state from _before_ it left, with nothing to
   correct it until the daemon changed state again. The signals are the only part worth skipping when
   nothing is listening; the state is what a returning host reads.
3. **The indicator was never retried** (`tray.rs`, S8). `Sni::start` ran only from `update`, and
   `update` ran only on a state change — so a session where this app and the panel start together and
   this one wins registered nothing, fell back to the text menu, and stayed there for as long as an
   idle daemon stayed idle. Retried on a 30-second cadence now, and the fallback tray is removed when
   the item comes up, because two indicators for one app is worse than either.
4. **A right click used to dismiss a stuck panel.** `lib.rs` documents the state where a compositor
   refuses the panel the focus it asked for, so no blur ever arrives to hide it — and the way out was
   a second click on the indicator, which reached `ContextMenu` and toggled it shut. With a menu
   published, that click goes to the host instead. `AboutToShow` dismisses the panel, which is what
   the host is about to draw over anyway.

**i. What a live session proved, and what it did not.** With the app running under Plasma 6.7:
plasmashell called `GetLayout(0, 1, [])` on the menu path **unprompted**, seconds after the item
registered — the importer exists, which is the whole question. `AboutToShow` answered, a synthetic
`Event(id, "clicked", …)` hid and showed the window through the same handler the fallback menu uses,
an unknown id was reported rather than silently swallowed, and pausing the daemon _from the menu_
moved the published rows to the paused set at the next revision with `LayoutUpdated` emitted and
honoured within a millisecond.

What is NOT proven is the pointer. A synthetic `Activate` still opens the panel with `Menu`
published, but that exercises an unchanged handler and would pass whatever a host decided to send —
so _that a real left click still reaches `Activate`_ rests on Plasma's `StatusNotifierItem.qml`, and
that Plasma renders these rows on a right click rests on its importer's source. Reading a host's
code is better evidence than reading a specification and it is not the same as watching it happen.
Two clicks close both gaps, and neither has been made.

**j. On GNOME the menu changes what a SINGLE click does, and the panel was always on the double.**
`10-tray.md` names GNOME-with-the-AppIndicator-extension alongside Plasma, and there is no GNOME
session here — but the extension is a JavaScript file that can be read, and it does not resemble
Plasma. `indicatorStatusIcon.js` puts `open()` — the `Activate` that opens the panel — on a **double**
click, gated on `supportsActivation !== false`; a single left click waits out the double-click
interval and then toggles the menu, and does that only `if (this.menu.numMenuItems)`. Before #252
that count was zero, because there was no `Menu` path and so no menu client: **a single left click on
GNOME did nothing at all.** Publishing the menu turns that dead click into an opened menu and leaves
the double click on `Activate` untouched, so the panel does not become less reachable there either —
it was never on the single click to begin with. Read, not run; the two clicks above are still owed,
and on that desktop it is three.

---

## A pass that failed (#246)

## 90. The seventh state, and the four places that were quietly answering `idle`

`derive_state` checked `paused`, then an auth-shaped `last_error`, then `syncing`, then never-synced,
then `pending_changes > 0` — and fell through to `Idle`. Every one of those is a state the daemon is
_in_; a daemon whose last pass failed for any non-auth reason is in none of them. So a remote list
that timed out, a `proton-drive` binary that was not on the `PATH`, a transfer that errored — all of
them drew **`Everything is up to date`**, which is the app's strongest all-clear and the one state
where it can be false.

The daemon had always said so. `reconcile_blocking` sets `self.last_error` on the failing arm and
`record_status_history` writes it into the history; the reply carries the string; `proton-sync`'s CLI
prints it. A grep for `last_error` across `gui/src/js` found one reader, and it was S7's merge
watcher. This was never a missing capability — it was a field nothing on the window read.

**Not a screen's bug, so not a screen's fix.** §67a is the same shape one state over and was fixed in
`heroStateOf` alone, because `authExpired` was already a `DaemonState` and only S1 was dropping it.
Here the derivation itself was short a state, so the fix is a seventh variant and the surfaces
collapse it the way they collapse the other six.

### 90a. Where it ranks, and why each neighbour is load-bearing

Three placements, each of which is a different wrong screen if moved:

- **After `syncing`.** `last_error` is cleared only by a pass that SUCCEEDS, so it outlives the
  failure it describes and is still set while the next pass runs. Read before `syncing`, a daemon
  that is actively retrying would be pinned to its last bad pass for the length of the retry.
- **After `FirstRun`.** A machine that has never synced still gets the onboarding takeover. Only
  reachable if the history sidecar is missing — `record_status_history` runs on both arms of a pass,
  so a failed first pass leaves a non-empty history and is `Failed` rather than `FirstRun` — but when
  both could apply the wizard is the better answer.
- **Before `pending_changes`.** A failure with a queue behind it is still a failure; the queue is why
  it matters, not a reason to call it `Running`. This one has a second copy in `heroStateOf`, which
  calls a non-empty queue `syncing` on its own — so a fix in `derive_state` alone would have replaced
  `Everything is up to date` with `Syncing 5 changes`, which is the same false all-clear one word
  further on. Both orderings are pinned, in both languages.

### 90b. Three other surfaces were answering `idle` for the same reason

Found by asking what else keys on the state rather than by testing the screen:

- **The header chip** (`chipFor`) has an arm per state and a fall-through, and the fall-through is
  `idle`. It would have read `idle` in the corner of a window whose hero said the last sync did not
  finish. It reads `sync failed` — the daemon's own word for it, the message
  `record_status_history` writes. **Not S5's word**, which an earlier draft of this claimed: that
  list labels every failed row `Couldn't reach Proton Drive`, the one sentence §90d rules out for
  this state. Pre-existing S5 wording, filed rather than absorbed (#258).
- **The tray panel** (`screens/tray.js`) has a `default` arm and the default is the settled copy, so
  the tray would have said `Up to date` while the window said otherwise. The panel takes the struck
  form and the failed sentence; the menu takes `unreachable`'s rows, because this is the one struck
  state where `Try again now` is unambiguously a working control — the daemon is ANSWERING, so
  `syncnow` reaches it. Same table, same reasoning, in `tray_menu.rs` for the two native menus.
- **The onboarding latch** (`routes.js`) releases on a list of reachable states, and `failed` had to
  join it. This is the half that inverts: before the state existed, a failed first pass derived to
  `idle`, released the latch, and handed off to the false all-clear — the bug. Adding the state
  without adding it here would have fixed the sentence and trapped somebody in a two-step wizard that
  cannot put `proton-drive` back on the `PATH`.

`counters_unknown()` stays false, unlike `Unreachable` and `FirstRun`: the reply is the daemon's own
and its numbers are real. Blanking them to em-dashes would claim not to know things it just said.

### 90c. The daemon's string is a block BELOW the hero, and that is not a styling choice

`14-behaviour-and-state.md`'s testing checklist ends on _"Daemon error strings shown verbatim, never
paraphrased"_ and its error table gives the shape for the rehearsal that could not run (§76, §79f).
This is the same treatment for the pass. But the hero is a fixed 394px block that CENTRES its column
— that is the property the whole screen rests on, and checklist item 2 is _"hexagon does not move
between states of the same screen"_ — so a fourth line inside it moves the mark by half a line in one
state and no other. The quote goes in the block below, where the transfer columns go while syncing
and the `flex:1` spacer goes otherwise; the hero's own four children are unchanged.

Measured at 1040×764, headless, against `settled` and `paused` as the reference: the mark's box is
`top:82 left:436` in all seven renderings, including a 10 KB error and a 10 KB error with no
whitespace in it.

**The cap is `max-height:100%`, not a number, and the number is what the measurement caught.** The
three sibling blocks (`.pl-failed-error`, `.ob-working-error`, `.pass-error`) can afford a fixed cap;
this region cannot, because an attention band appearing under it takes the space away — 268px becomes
123px with one conflict and one deletion waiting. A fixed 132px there overflowed the region by 9px,
which the wrapper's `overflow:hidden` then cut off: the box lost its bottom border and the last line
of a string that nothing could scroll to. A decision and a failed pass are both ordinary and they
coexist by design (_"needs-decision is additive, not exclusive"_), so that state is not a corner.
With the percentage and 16/24px of wrapper padding: 10 KB ends at y=690 against a footer at y=714,
scrolls inside its own box, and the band case clips nothing.

**A failed pass with no string falls back to the spacer**, rather than drawing an empty quotation.
Nothing sets `failed` without a string — `derive_state` keys on it — but the screen takes its props
from a caller and #247 is the standing lesson that a block rendering nothing passes every gate.

### 90d. The copy is S1's, and it does not borrow the two sentences that nearly fit

`The last sync didn't finish` / `Nothing is lost. 4 changes are waiting and will go on the next try.`
`MAIN.failed` is exempted in `copy-gate.mjs`; no `2a` frame draws this state. The sub-line needs no
exemption and cannot have one — it is a template, which the gate's walk never collects, and §90e is
the check that now says so out loud.

- **Not `Can't reach Proton Drive`** (the deck's one sentence for this family, `TRAY.unreachableTitle`).
  It is already spoken by a different state, and it is a claim we cannot make: a pass can fail with
  Proton perfectly reachable — a full disk, a binary that moved. The quoted string underneath would
  contradict the headline over it.
- **Not S4's `Nothing has been touched`**, which is true of a rehearsal and false of a pass: the
  engine's checkpoint commits mean a half-finished pass DID move files. What it never does is record
  a side effect that did not happen, and the failed action re-plans. So the promise is about loss.
- **`0` drops the count clause**, on `TRAY.unreachableBody`'s rule but for a different reason: here
  the number is known and genuinely zero (a pass driven from Proton's side has an empty watch queue),
  and `0 changes are waiting` reads as an all-clear in the one place that must not give one.

### 90e. Two gates that were counting states by hand, and now read them off the enum

Both are the same shape as the bug: a list that could not notice something new.

- **`tray-view.test.js`** enumerated the six daemon states as a string literal, so a seventh added on
  the Rust side would have been tested by nothing — and `trayMenu` THROWS on a key it does not know,
  which in a borderless panel with no devtools is a tray that silently stops opening. It now reads
  the variants out of `gui-core/src/state.rs` and lowers them the way `serde(rename_all)` does.
- **`icons.rs`**'s glyph test enumerated them too. It walks `tray_menu::ALL_STATES` instead, and that
  const is now pinned by a match over the enum — the only construction available in Rust that a later
  author cannot satisfy by doing nothing.

And one gate audited against itself: `copy-gate.mjs`'s `NOT_DRAWN` is a `continue` before the lookup,
so an exemption that names nothing is never contradicted — its own header says as much and stopped
there. `MAIN.failedSub` was nearly added to it, which would have excused a TEMPLATE the walk never
collects while still subtracting one from the printed total. Exemptions are now checked to name a
string the gate would otherwise look at; the 44 that were already there all do, and this section's
own entry makes 45.

### 90f. The release set had a second copy, and the second copy was a kill switch

§90b above says adding `failed` to `nextOnboardingLatch`'s release list is what stops the fix
trapping somebody in the wizard. It was necessary and it was not sufficient, and a review of the
merged change found out why.

`render()` kept its **own** copy of that list — `const reachable = st === "idle" || st === "running"
|| st === "paused" || st === "authExpired"`, under a comment reading _"EXACTLY
`nextOnboardingLatch`'s RELEASE SET"_ — and it gates the sticky `onboardingFailure` that
`failOnboardingMerge` sets when a first sync fails. The latch expression is

```js
onboardingStage !== null ? false : onboardingFailure ? true : nextOnboardingLatch(…)
```

so `onboardingFailure` **short-circuits the function entirely**. The two lists disagreeing therefore
did not produce a mismatched screen, which is what a second copy usually costs. It made the new arm
in `routes.js` dead code on exactly the path it was written for: a failed first sync latched the
takeover shut, undismissable, for as long as the daemon kept failing — the inverse of the hand-off,
and worse than the `Everything is up to date` it replaced.

The S1 failed screen this whole section is about draws for one poll and then the wizard returns over
it. Three texts asserted the opposite while it did: the comment above `reachable`,
`failOnboardingMerge`'s _"a reachable daemon that failed a pass is the main screen's business"_, and
§90b.

**The fix is not to make the copies agree.** `releasesOnboarding` is exported from `routes.js` and
both callers ask it; the second list is gone. `onboarding-latch.test.js` now also reads `app.js` and
fails if a hand-written state list comes back near `reachable` — the same construction the tray test
uses against `state.rs`, and for the same reason: the thing to defend is not this bug, it is the
shape.

That shape is the one this codebase keeps paying for (§67c, §64, the `severityOf` band). Worth
naming what made this instance invisible: **the duplicate was in a caller of the function it
duplicated**, so grepping the fixed function's own file and tests showed nothing, and every gate
stayed green because no gate renders the onboarding takeover against a failed daemon.

### 90g. Three more, all comments, and one of them was a count

Found by the same review, all in this section's own prose or the diff's comments — which this
codebase treats as defects rather than tidying, because a comment that states a falsehood is read as
evidence:

- **`chipFor`'s justification named the wrong screen.** `sync failed` is the daemon's message and
  the chip is right; the clause claiming it is _"what S5's passes list draws"_ was not — `passRowFor`
  labels every failed row `Couldn't reach Proton Drive`. Which is the sentence §90d rules out for
  this exact state, on the exact same `last_error` field, so S5 says the thing S1 refuses to. That
  is pre-existing S5 wording and is #258 rather than something to absorb here.
- **`subOf`'s failed arm pointed at `failedBlock`,** a function that does not exist. The block is
  built by `fillFailed`. That cross-reference is the only navigation between the two halves of the
  hexagon-does-not-move rule, so it pointed at nothing from exactly the place that most needed it.
- **§90d said both new strings were exempted in the copy gate.** Only `MAIN.failed` is;
  `MAIN.failedSub` is a template, and §90e — four paragraphs later, in the same section — is the
  note explaining that a template needs no exemption and that the new check now _rejects_ one. And
  §90e's own figure counted the exemption this change added as pre-existing: 44 were already there,
  not 45. Both corrected above. The same slip as §88's, one section on: a change invalidates the
  numbers quoting it, including the ones written in the same commit.

## The light theme (S10)

## 91. The `12a` frames' ground truth was the page's, not the frame's

§58b measured this and handed it here: the prototype draws all sixty frames on one document, that
document's wrapper carries `color:#F2F4F7`, and a node inside a `12a` frame that declares no colour
of its own was extracted as the dark text tier. An app that correctly inherits `#14161A` failed on
every one — **142 failures across the three compacts, one class, none of them a real difference** —
which is why those three were mapped, measured and taken back out, and the four light windows were
never mapped at all.

Nothing about that was a mapping problem, and the fix is in `extract.mjs`. Each node now records
`fromPage`: the properties whose value came from outside the frame root. `assert.mjs` declines to
compare those on a `12a` frame and **prints how many it declined**, per frame, every run — 628 of
them, on the theme with the least drawn ground truth, which is a number a reader has to be able to
see before "0 failures on a light frame" means anything.

**Five properties, not "everything inherited".** `font-family`, `font-size` and `letter-spacing`
inherit from the same wrapper and are ground truth in both themes; wildcarding them to fix a colour
problem would delete real light assertions. The list is `color` plus the four `border-*-color`,
which is exactly the failure class §58b measured — and the two halves have different rules.
`border-*-color` does not inherit, it defaults to `currentColor`, so its condition is "this node
declares none of its own, **and** its `color` comes from the page". A node that sets `color` and
leaves its border alone has a border colour the frame really does specify.

**Recorded for all 51 frames, applied to 8.** On a dark frame the inherited `#F2F4F7` is
accidentally correct — the app inherits it too — so it is a real comparison and dropping it would
have traded a fixed light theme for a weaker dark one. Same fixture, different reading. Verified
rather than assumed: re-extracting with `fromPage` and running the gate produced **byte-identical
output** to the run before it — 43/51 frames, 79,330 assertions, 0 failures.

With that fixed the seven product frames map to their dark twins' `fids` rather than restating them.
The two trees are the same tree drawn twice — 26/26, 60/60, 75/75, 63/63, 12/12, 36/36, 13/13, same
keys in the same order with the same text, measured — so a hand-written light table would be a
second copy of one that already exists. `check-fixtures.mjs` grew a fifth check that fails the build
the day a `sameAs` pair stops agreeing, which is what makes inheriting the mapping sound rather than
convenient. **51/51 frames now carry a mapping, 94,299 assertions.**

**The gradients were the one structural edit the doc warned about, and they were already right.**
`12-light-theme.md` and the plan both flag "SVG gradient stops must be theme-aware — easy to miss";
§59a settled it three tasks early by driving the stops from CSS variables rather than duplicating the
defs, and verified it by sampling pixels. This is the first time a drawn light frame has been in a
position to say so: `12a Syncing light` carries the two-gradient hexagon, its four `stop-color`s are
compared as computed values, and all four land on the light palette's own `--up-from`/`--up-to`/
`--down-from`/`--down-to`. One defs pair, both themes, asserted rather than sampled.

### 91a. Three defects, and all three were invisible in dark

Every one is a token pair that coincides in dark and parts in light — the shape `tokens.css`'s own
header warns about at length, found for the first time by a frame that could see it.

- **The permanent column's eyebrow wore the decision hue.** `deletionColumn` gave both columns
  `eyebrow-decision`, which is right for `Recoverable · Proton Drive` and wrong for
  `Permanent · this computer`: `--decision-text` and `--destructive-text` are both `#FF9C9C` in dark
  and `#BE123C` against `#B91C1C` in light. The dot beside it already followed severity; the label
  now does too (`.eyebrow-destructive`).
- **`--btn-destructive-disabled-bg`'s light value was a derivation, and the derivation was wrong.**
  `.08`, from dark's `.1`. `12a Deletions light` draws the disabled `Delete` at **`.07`** — the
  first frame in the bundle to draw it in light, and the tokens.css note said as much without
  anyone having looked.
- **The deletion card's kind had no token of its own.** `a folder` and `4 KB` were `--text-4`, which
  is `#828B98` in dark — correct — and `#4B5563` in light, where both drawn cards put the node at
  `#6B7280`. The facts strip _inside the same card_ does map to `#4B5563`, which is what
  `12-light-theme.md`'s text-on-tint rule asks for, so this is not a card-versus-not-card
  distinction: it is one dark hex doing two jobs. Now `--deletion-kind`.

  **The reading a designer could overturn**, stated because two sites is not many: §9 called a
  single node against sixteen a drawing inconsistency, and these two cards are one component drawn
  twice, so one slip yields two nodes. What decides it the other way is that §9's node had sixteen
  counterexamples **at its own role** and this role has none — these two are every instance of it in
  the bundle, and they agree.

### 91b. Which light values a drawn frame measures, and which are still chosen

Walking the eight light frames against their twins gives a verdict per token rather than a promise:
of the **106** themed tokens that carry a colour, **73** now have their exact (dark → light) pair
observed at a node in a drawn light frame, and **33** do not.

The 33 are not a to-do list, they are the honest edge of what is drawn — the notification banner (5),
the diff panel (4), the compact deletion rows (4), the `Never ask` card (3), the seam (2), the warn
band (2), the armed confirmation, the inert glyph outline, the scrim. No `12a` frame draws any of
them, so they stay **CHOSEN**, derived from the ramp around them, and their tokens.css notes now say
so instead of saying "S10 confirms".

**One of those notes claimed a measurement that never existed.** `--destructive-row-bg`'s light
value was justified as "read off a frame rather than derived — `--compact-permanent-bg` is this same
alpha and steps `.05 → .03` in `12a Compact needs light`". `12a Compact needs light` draws no
deletion row: it is thirteen nodes — a mark, a headline, a sentence, `Review them`, `Later` — and
the only tint on it is the button's `--decision-btn-bg`. `--compact-permanent-bg`'s own note, four
rows further down the same file, says the opposite and is right. So one derived value was cited as
evidence for another, from a frame that draws neither. Both stay chosen, and the note says which.

**The method's own limit, since it decides 73 of the numbers above.** It matches a token by its
(dark, light) VALUE PAIR, so two tokens carrying the same pair under different names are
indistinguishable to it — which is the doctrine `tokens.css` is built on, not an accident.
`--btn-primary-disabled-text` reads as "observed" only because `#6D7783 → #9CA3AF` appears at the
queued-row arrow that §9 already ruled a drawing inconsistency; nothing draws a disabled primary
button in light. `--hex-paused-track` and `--hex-paused-bars` read as observed for the plainer version
of the same reason — they share both values with `--hex-settled-track` and `--text-3` — and no light
`10a Paused` exists. All three stay on §15's unverified list, which is why that list is 7 rows and not
4: **the census can confirm a VALUE and never a SITE**, and §15 is about sites.

### 91c. The contrast gate, and the two thresholds it needed instead of one

The seven screens with no light frame get `check-contrast.mjs`, which reads every text node in both
themes and compares it **to itself**, because there is no drawn artefact to compare it to.

A fixed WCAG floor was the obvious design and it is wrong here: `--text-5` is 4.33:1 in dark and
recorded as deliberate, and light draws a disabled `‹` at 1.76:1 on a frame that IS drawn and IS
asserted. A gate that fails where the design is right gets an exemption list and then gets ignored.

Parity alone is wrong too, and the first run measured that rather than arguing it: **the accents drop
hardest of anything in the design and drop correctly.** `--down-label` is 10.89:1 in dark and 5.05:1
in light — 46% of it, both values drawn, both asserted. Everything below 0.55 parity is an accent
arrow or a diff-gutter numeral, and the dimmest of them is still 4.67:1.

So it takes both, and the two populations are disjoint on opposite axes: nothing under 3:1 has a
parity worse than 0.62, and nothing under 0.5 parity is dimmer than 4.74:1. A token left at its dark
value lands in the corner neither occupies — `--up-to` unmapped measures **1.62:1 against 11.45:1, a
parity of 0.14**; `--decision` unmapped, 2.62:1 against 7.09:1, 0.37. `S10_CONTRAST_POISON=1` puts an
accent back to its dark value and the run must exit 1 naming the eyebrows that carry it — it does,
with 43 findings against the clean run's 0 — because thresholds placed in a gap measured from passing
data are exactly how a check that can only ever pass gets built.

**Two artefacts of the tool itself, found by reading its own output rather than by trusting it.** Its
alpha compositing returned `a: 1` unconditionally, which is right only over an opaque layer — the
design stacks tints two and three deep, and `Move to Proton's Trash` (`rgba(190,18,60,.06)` inside a
`rgba(190,18,60,.03)` card) came out as solid `#BE123C`, the same colour as its own label, reported
as 1.00:1 **in both themes**. And it read every SVG node with a `fill`, so 21 hexagon bodies carrying
`fill: var(--surface)` to mask the seam were reported as text that had vanished — being exactly the
colour behind them is that node's entire job. **61 findings became 35 became 0**, and 26 of the 61
were the gate accusing the design of its own bugs: 21 masks and 5 stacked tints.

**And the wildcard did not blunt the style gate**, which is the thing a reader should want checked
before believing 51/51. Moving a DECLARED light value by two hundredths of an alpha —
`--decision-card-border` from `.28` to `.30`, a value `12a Deletions light` draws — fails that frame
on exactly four assertions. 628 comparisons are declined; the ones the frame actually makes are as
exact as they were.

**What it does not cover, stated rather than implied:** strokes. A glyph whose stroke is mapped to
the wrong end of its ramp is invisible here, because a track like `--hex-syncing-track` is drawn a
shade off the surface on purpose and a legibility gate has no opinion about a track. `assert.mjs`
compares strokes exactly on eight drawn light frames, which is the stronger check — it is the seven
undrawn screens that have only this one.

---

## Verifying the tray with a pointer (#187)

## 92. Three of them only a hand could find, and one of them cut the tray's two most important words off the bottom

`10-tray.md`'s definition of done ends with a clause no gate can execute: _"verified on GNOME
(AppIndicator extension) and KDE Plasma."_ §89i said so in as many words — "two clicks close both
gaps, and neither has been made" — and left them owed. This is the section that makes them, on a live
Plasma 6.7/X11 session, with `xdotool` driving XTEST so the pointer is the X server's own and not a
D-Bus call standing in for one.

**Both clicks land, and the bus says so.** A real left click on the indicator arrived as
`Activate(3132, 2112)` from plasmashell, and the panel opened centred on it with its bottom edge on
`_NET_WORKAREA`'s — clamped into the work area and therefore opening UPWARD, which is what a bottom
panel needs and what the spec's fixed `top:40px; right:16px` would get wrong on every Plasma desktop.
Re-measured at all five panel heights (302–440 logical px) and every one of them anchored there. A
real right click produced `AboutToShow` → `Event` → `GetLayout` on `/StatusNotifierItem/Menu` and
Plasma drew the rows — the settled five, with §82k's em-dash fold intact: `Close window — keeps
syncing`, `Quit — stops syncing`. Clicking a row sent `Event(1, "clicked")`. §89i is discharged for
KDE; **GNOME is not**, and stays owed for the reason it always had — there is no GNOME session on
this machine, and §89j's reading of `indicatorStatusIcon.js` is reading, not running.

### 92a. The panel measured itself inside the window that measurement sets

`reportTrayHeight` measures the panel and `panel.rs` sizes the window to the answer. `#app-root` is
the shell's `height:100vh` flex column, and a flex item shrinks — so the measurement was
`min(content, viewport)`, capped by the window it had just set. **One short measurement and the panel
could never grow again**, in any state, for the life of the window.

Which is what shipped: on a cold webview profile the settled panel came up **302px tall where it
draws 365**, and the two rows past the cut were `Close window — keeps syncing` and `Quit — stops
syncing` — the pair `10-tray.md` calls _the single worst misunderstanding a tray app can cause_ and
requires the build to keep. Switching the daemon to a state whose panel is 440px tall changed
nothing: still 302, on every poll, for as long as it was watched. The latch is the finding; which
transient measurement arms it (an early frame before the window settles at its built size) is not
worth pinning, because any of them is fatal and a cold profile reproduces it every time.

**No gate could have caught it and that is structural, not an oversight.** `assert.mjs` opens every
frame at 1040×764, so a 362×365 panel has 399px of slack and nothing can compress it. The shipped
window has no slack at all — it is exactly as tall as the panel said it was. So the harness now
re-measures **all 11 compact frames in a 200px window** and requires the drawn height back; poisoned
by reverting the fix, it fails all eleven.

`flex:none` on `.compact-panel` would fix it and is not available: the gate compares `flex-shrink`
against the drawn node, which is `1` — `shell.css` already carries two comments saying exactly that
about `.menu-btn` and `.chip-dot`. So the container gives up its height instead
(`#app-root.is-panel-surface`), applied by `panelSurface()` at both mounts. Both, and they have to
stay in step: the gate only ever renders the preview one, so dropping it from `mountTrayPanel` alone
would leave the gate green and the app broken.

### 92b. `set_focus()` is a request, and KWin was refusing it

The click that opens anything in this app goes to **plasmashell**, not to us. So when we ask for the
focus, the timestamp the request carries is whatever GTK last saw an event with in this process —
older than the click, which is precisely the shape focus-stealing prevention refuses. The panel came
up carrying `_NET_WM_STATE_DEMANDS_ATTENTION`, the WM's marker for _this window asked and I said no_.

Two failures, one seam:

- **The panel lingered.** `lib.rs` hides it on a blur that FOLLOWS a focus — deliberately, so a
  compositor that refuses focus cannot make it flash and vanish (§82, bug 4). Refused focus means no
  focus, so no blur ever counted: an always-on-top borderless popover stayed over the user's work
  until they clicked the indicator again. IMPLEMENTATION-PLAN §6's second sub-risk, and not on an
  edge case — on the ordinary path, since _another app holds the focus_ is the normal state of a
  desktop. That file's own consolation ("Esc, any menu row, and a second click all still dismiss it")
  was two-thirds true: **Esc could not reach a window that had no keyboard focus.**
- **`Open Drive Sync` did nothing.** With the window already open behind others, the row raised
  nothing and focused nothing. The stacking order was identical before and after.

`focus::present` fixes both by asking with a timestamp `gdk_x11_get_server_time` supplies, which is
the X server's own clock rather than anything this process guessed. **It runs from a `glib` idle
callback**, and that is not a flourish: `WebviewWindow::show()` posts a message to the event loop
rather than calling GTK, so `gtk_window().window()` is still `None` on the line after it — the first
version took the not-realized branch every single time and changed nothing. It is also the guard
against the one outcome worse than the bug: `gdk_x11_get_server_time` waits for a `PropertyNotify`
that only a mapped window will send, so an unmapped one would hang the GTK main loop. The plain
`set_focus()` stays where it was and the stamped present is added after it, so Wayland, an unrealized
window and a server that returns no time all behave exactly as before.

Measured after, on the same three tests that failed before: the panel takes the focus, clicking away
hides it, Esc hides it, a second click on the indicator hides it, and `Open Drive Sync` moves the
main window above the one that was covering it. `gtk` and `gdkx11` are new _direct_ dependencies and
no new build — Tauri already compiles both, and `WebviewWindow::gtk_window()` hands back a
`gtk::ApplicationWindow`, so the versions must be the ones Tauri resolved.

### 92c. Nine deviation rows were pointing at the issue that builds the thing

`10a In situ`'s nine recorded rows all cited **#187**, which closes with S8 — leaving nine entries
naming a closed issue and no gate able to notice. They are two different kinds of thing and are now
recorded as such:

- **Four are a design tie**, carved out as **#261**: `10-tray.md` asks the tray form for
  `rgba(255,255,255,.1)` because it floats over a desktop, `10a In situ` is the only frame drawn that
  way, and the four gated standalone panels all draw `#23262D`. §58d ruled for the four with only the
  four in view; the doc's reason is still a good one, and the tie is the design's to break.
- **Five no issue will ever close.** The drawn panel is placed on a desktop mock with
  `position:absolute; top; right`; the shipped one is a window, and its placement lives there —
  measured above, against the click, clamped into the work area. `known-deviations.mjs` grows a
  `structural` tag for exactly this, printed under its own heading and still bound by the
  must-still-fail clause, so it cannot become the escape hatch that file's first paragraph forbids.

Splitting them exposed a lookup bug one line long: the printout matched a row on `frame|key` alone,
and `10a In situ · div[1]` carries both sets. It printed the right issue only because all nine
agreed; the moment they stopped, four would have been labelled with the fifth's answer.

### 92d. The raise fix landed on one of the two `Open Drive Sync` paths

Found by asking which callers of `focus::present` had actually been clicked. Two, and only one had
been: the native menus route through `tray::show_window`, and the panel's own row went through a
**second copy of the same three lines** inside `commands::tray_action`. So after 92b the right-click
menu raised the window and the panel's own row — the more likely of the two to be used — still left
it exactly where it was. The copy is deleted; `tray_action` calls `show_window`.

That is also why `focus::present` hops to the main thread itself rather than documenting that its
caller must. `set_focus` and `show` are Tauri proxies that post to the event loop and can be called
from anywhere, so every existing caller is wherever it happens to be — `tray_action` is an **async
command on the runtime's pool**. `glib::idle_add_local_once` asserts ownership of the thread-default
main context: called off it, it does not misbehave, it aborts the process. A doc comment asking
callers to be on the main thread would have been a rule that is obeyed until the first caller who
has not read it, and `panel::hide` and `panel::resize` already carry their hop for the same reason.

### 92e. `— undefined`, on the one run that matters

Copilot, reviewing #262: the stale-deviation printout interpolates `d.issue`, and a `structural` row
has none, so the line the gate prints on the day a structural row stops failing would have read
`10a In situ · div[1] · top — undefined`. Verified by pinning that row's `detail` to a value it
cannot match, which is what makes it stop matching; it now reads `— structural`, and the two words
mean different things to whoever reads them. An issue means the capability landed, so delete the row.
`structural` means a property the app could never carry now agrees with the drawing — either the app
changed or the frame was re-extracted, and both want reading before anything is deleted.

---

## A failed pass names its cause only when it has one (#258)

## 93. The row asserted the one cause it could not check

`passRowFor` labelled every failed pass `Couldn't reach Proton Drive` on the sole test
`entry.last_error != null`. The label is the deck's, read off `6a Activity passes`, and it is right
for the pass that frame draws — a connection timeout. It is not right for the field it keys on:
`last_error` is whatever `reconcile_blocking` caught, so a full disk, a `proton-drive` binary that
moved and a local file that could not be read all arrived on screen as Proton being unreachable —
**directly above the daemon's own string, quoted verbatim, saying otherwise.**

#246 had settled the identical question one screen over and settled it the other way (§90d): S1's
failed hero is `The last sync didn't finish` precisely because naming Proton is a claim the data does
not support. So the two screens disagreed about the same field, and the one that disagreed loudest
was the one with the drawing behind it.

**The split is by cause, not by presence.** `ACTIVITY.passes.unreachable` survives for an error whose
own words are about reaching Proton; everything else gets `Didn't finish`, which names nothing. That
keeps `6a Activity passes` rendering exactly what it draws — its error is
`proton-drive: connection timed out after 60s` — so the style gate never moves, and the new string is
exempt in the copy gate for the reason that made the bug possible: no frame draws a failure of any
other kind.

**Not `looks_like_auth_error`, which is what the issue proposed.** It is the wrong split for this
label twice over. An expired session means Proton was _reached_ and refused you, so the auth-shaped
subset is exactly the one that must not say `Couldn't reach Proton Drive`; and the frame's own error
does not match it, so following the suggestion literally would have flipped the drawn row to the
neutral label and failed the gate on the frame it was meant to be faithful to. An issue naming a
mechanism is a hypothesis about it.

**What the needle list is, and what it is not.** `last_error` is mixed provenance — some of it the
engine wrote (`proton-drive {operation} timed out after {duration}`, `src/proton.rs`) and the rest is
the CLI's stderr passed through — and nothing classifies it, so this is a pattern match on prose,
the same compromise as `gui-core`'s `looks_like_auth_error` and with the same ending (#103/E6, the
daemon classifying its own failures). It is deliberately tight, because the two errors are not
symmetric: a miss labels a genuine outage `Didn't finish`, which is quieter than the truth and still
true, while a false hit is the bug this section is about. Nine phrases, each transport vocabulary a
local failure has no reason to use, pinned in both directions by `activity.test.js` — including the
five real failures that must NOT match.

## Navigation everywhere, a Home door, and a search that searches (2026-08-13)

## 94. Three product decisions taken against the drawings

The frames are a design, and on these three points the product decided otherwise. Recorded here
rather than argued in a commit message, because every one of them contradicts something this file
already measured.

**The doors are drawn on every screen but the takeover.** §40 measured the split — 13 in-scope 1040
frames carry the four doors, 6 carry a footer action bar, never both — and read the consequence
correctly: on Settings, Plan and onboarding there was no navigation at all, so the app mark had to be
a home affordance. That is the part that did not survive contact: a user on Settings sees the
navigation vanish and has no reason to know the app mark is a way out. `app.js` now draws the nav
under the action bar on those screens, and only the onboarding takeover keeps none — on a fresh
machine there is nowhere to navigate to, and the flow's own step buttons are the way through.

It costs 50px of content region on two screens, and every one of the four recorded deviations is that
50px arriving somewhere measurable: `.settings-content` scrolls rather than pushing the footer off
the window (a `flex:1` item with no `min-height:0` refuses to shrink below its content, so the whole
window scrolled and the footer went with it); the folder pair's seam is pinned 60px off each end of a
tab that is 50px shorter; the plan list fits eight of its nine drawn rows and scrolls for the ninth.
They are tagged `decision` in `known-deviations.mjs` — a third class beside `structural`, for a
departure that no capability closes and no issue tracks, and it carries the must-still-fail clause
unchanged.

**Home is a door.** `navigate` used to answer a click on the lit door by going back to the main
screen — IMPLEMENTATION-PLAN §3.3's assumption, and on Settings and Plan the only way back at all.
Two things were wrong with it. A tab that toggles is not a tab: clicking `Activity` while on Activity
left the screen you were looking at. And it was silently destructive — re-entering a screen resets
it, so the toggle discarded a half-typed lookup and an in-flight rehearsal. So `FOOTER_ORDER` gained
`main` at its head, labelled `Home`, the lit door is now a no-op, and the app mark stays a home
affordance as well (a second route home is redundant, not wrong). The frames draw four doors and none
of them lit on the main screen; the app draws five and lights Home there.

The fid mapping is where that stays honest. `doorKeys` maps the app's door _i_ to the frame's
`span[i-1]` and answers `null` for door 0, so every drawn door keeps its identity in the gate and the
one the frames do not draw is not compared against a node that does not exist.

**The lookup field searches.** G21 recorded the gap and #234 tracked it: `path_sync_status` opens the
index AT the string it is given, so `spec.md` — the query `7a File lookup` itself draws — answered
`No file by that name in your sync folder` for a file sitting at `docs/spec.md`. The frame drew a
search; the screen shipped an exact-path lookup, and the honest miss sentence made it look
deliberate.

`search_files` (gui-core `index_read::search_records`) matches an exact path, a whole name, a
trailing run of components, then any fragment, ranked in that order and stable across runs. It is a
full table scan — the index is keyed by path and has no name column, so a name search cannot be a
point query — which is why the command is `async` + `spawn_blocking` (#142/#143: a stalled GTK main
loop aborts WebKitGTK). A pasted absolute path is reduced to the relative one the index stores, and a
leading `~` expands, because that is what someone pastes out of a file manager.

Three outcomes, and only one of them was reachable before: one match resolves straight to the
verdict; none is a miss (carrying the error separately, so a failed search never renders as a missing
file); several are listed for the user to choose from. The chooser is undrawn — no frame could draw
it, since the old lookup could only ever answer about one path — and it deliberately shows no
hexagon and no seam: both are claims about one file's standing, and over a list they would be a
verdict about whichever row they sat above. `ACTIVITY.matches`' plural arm is reachable for the first
time, and the count states the TOTAL rather than the capped list, so a search that matched 132 files
never reads as though it matched 50.

## Starting the daemon from the GUI (2026-08-14)

## 95. `unreachable` was two states wearing one name, and only one of them was Proton's

The design has one struck state and calls it _Proton unreachable_: `14-behaviour-and-state.md` puts
it "after a failed pass and retry", `10a Offline` draws `Can't reach Proton Drive` over `Try again
now`, and `11-notifications.md` groups it with an expired session and a full disk behind one icon.
Every one of those is about the far end of a sync.

`gui-core`'s `DaemonState::Unreachable` is not that. It is returned for exactly one thing — the
**control socket** did not answer: missing, refused, closed early, or a reply that would not decode.
Proton takes no part in that round trip. A daemon that is running and cannot reach Proton answers
perfectly well and derives `Failed` or `AuthExpired`, which have had their own sentences since #246.

So the state the app most often reaches after a reboot — no daemon — was drawing a diagnosis of the
wrong machine, and was offering `Try again now` to fix it. That row is `onSyncNow` in the window and
`ControlCommand::Syncnow` on both native menus: a control-socket round trip, sent down the socket
whose silence is the definition of the state. It could not have worked, and it did not fail
visibly either — it re-asked, failed the same way, and redrew the same screen.

**The window and both tray menus now offer `Start the sync service`**, wired to the `start_service`
command that existed and, until now, had exactly one caller: onboarding's `Start the first sync`. The
capability was complete; nothing outside the wizard could reach it. It prefers the user's systemd
unit and falls back to spawning `proton-syncd` against the GUI config, which is `restart_service`'s
own shared path (`start_service_impl`) rather than a second copy of that decision.

Three consequences worth naming.

**The row set was renamed, not extended.** `unreachable` held both states and the name matched the
wrong one, so it is `outage` (`Failed` — Proton is out, the daemon answers, retry it) and
`notRunning` (`Unreachable` — start the service) in both languages. A shared id whose meaning
depended on the current state was never an option: `tray_menu.rs` is built on the id being the
action, precisely so a menu left open across a state change dispatches what its label promised.
`START` therefore takes `dbus_id` 8 rather than overloading `tryAgain`'s 5.

**`TRAY.unreachableTitle` is now drawn and unspoken.** No screen renders `Can't reach Proton Drive`
any more; the three states that could mean it each say something truer. It stays in the deck because
`10a Offline` draws it and the copy gate checks the frame, and `copy.js` says so at the constant so
the next reader does not take its presence for currency. `unreachableBody` is still live — the
outage notification uses it where the cause genuinely is unknown.

**The failure is quoted, not swallowed.** `start_service` is the only command on the main screen that
REJECTS rather than folding its failure into a payload, and its message names which of the two ways
it failed ("no systemd unit … and no config file at …"). It rides `startError` into the block a
failed pass already uses. Without it the button is the dead control #227 and #224 record: pressed,
nothing visible, no reason given.

No frame draws any of this, and none could — the design has no state for a stopped service.

### 95a. Two things review found that every gate had passed

**A start failure outlived its subject.** `serviceStartError` had two writers, both inside
`startService`: set on rejection, cleared only by the NEXT click. Nothing retired it when the daemon
came up — and the routes that bring a daemon up mostly do not go through that function. The tray row
this same change adds starts the service entirely in Rust; Settings' restart has its own path; a
terminal has none. So the string survived into a later, unrelated outage and was drawn as _that_
outage's reason, in the block whose stated job is to be the account of why. `clearsStartError` is the
rule `app.js` already applied to `onboardingFailure` twenty lines above `mainProps` — "a merge that
failed against a daemon that then came up is not onboarding's problem any more" — applied to this.
It is deliberately narrower than that one's `releasesOnboarding`: the subject here is the socket
answering at all, by any route.

**Two of the new tests would have survived a revert.** Reverting all three window-side branches —
`headlineOf`'s and `subOf`'s `unreachable` arms and `quotedError`'s — left 322/322 green and the copy
gate at 0 missing. The tests named after those branches asserted properties of the _constants_
(`MAIN.notRunning` is not `TRAY.unreachableTitle`, says nothing about Proton), which no edit to the
screen can falsify, and a prop round trip through `mainView` for the other. They pinned the deck, and
the deck was never what changed.

`heroActionsOf` had the right shape already and its tests do fail on a revert; the fix was to give
the copy and the quoted block the same seam — `headlineOf`, `subOf` and `quotedError` are exported
and asserted through, and the revert now fails two tests. This is the file's own lesson from S5
arriving one screen later: a test over a function the caller does not reach proves the function, not
the feature.

## The plan screen's lists are bounded by the window (2026-08-14)

## 96. Seven files took the primary action off a window that cannot scroll

`5a Plan safe` is the one body on this screen whose height is the plan's length. A 300px hero, then a
row per file under each side count, then a `flex: 1` spacer — and the spacer is the only slack there
is. Measured at 1040×764, with the app's own fixture and the doors §94 put under the action bar:

| rows on the taller side  | document height | the four doors                    | `Run this sync`                  |
| ------------------------ | --------------- | --------------------------------- | -------------------------------- |
| 4 (the frame's own plan) | 764             | on screen                         | bottom edge at 699               |
| 6                        | 764             | on screen, and the spacer is gone | 699                              |
| 7                        | 797             | 33px below the fold               | 732 — the last row that keeps it |
| 8                        | 830             | 66px below                        | 765, 1px below the fold          |
| 44                       | 2018            | 1254px below                      | 1953, 1189px below               |

The window is a fixed, non-resizable 1040×764 with no scrollbar of its own, so past six files the
whole window scrolled: the four doors go first and the screen's primary action follows one row later.
A plan that moves seven files is an ordinary plan. #267.

**Scrolled, not capped, and the block that shrinks is the seam block.** 02-shell.md states the rule
for exactly this ("where a list can genuinely exceed the space, use `overflow-y:auto` so the clip
reads as a scroll region"), and this screen's other body already follows it — `.pl-rows` became
unconditionally scrollable under §94, having first shipped a `ROWS_THAT_FIT` constant for a height
that was not fixed. A cap here would be that constant again: six rows fit under a 300px hero today,
five would if the footer ever grew the optional line 02-shell.md reserves beneath the doors. Letting
the hero give up height instead was considered and refused — it centres a mark, a headline and a
sentence, and shrinking it moves all three for a reason the user cannot see.

So `.pl-seam-block.is-bounded` (the safe body only — `5a Plan`'s block is untouched) may shrink, its
grid carries that bound down to the columns, each column is a flex column so the squeeze lands on the
list rather than on the 42px count above it, and each list scrolls. Seven assertions on `5a Plan safe`
move with it, all tagged `decision` in `known-deviations.mjs`: `flex-shrink` on the block, `display`
and `flex-direction` on both columns, `overflow` on both lists. The frame draws three files and two,
so it had no occasion to bound anything.

Two more measurements taken in the same window, on the same screen, neither of them in #267:

**A long path grew the document sideways.** `.side-path` ellipsises, but only inside a column that is
allowed to be narrower than its content: a grid item's automatic minimum is `auto`, so a 207-character
path widened the `1fr` column, the grid and the document to **2198px** in a 1040px window. `min-width: 0`
on `.pl-side` is the whole fix, and it applies to both frames.

**The failed rehearsal quoted a daemon without bounding it.** The fourth body no frame draws
(§76) prints the daemon's exact string, which is right — voice rule 4 — and unbounded, which is not:
a 10 KB stderr wrapped to **1853px** of column and painted its own tail over the footer. §79f named
this in a subordinate clause when onboarding borrowed the block — "the error block caps its height
and wraps anywhere, which `.pl-failed-error` does not" — and the clause outlived the note: the
onboarding block was bounded, `.main-failed-error` was bounded under §90, and the block both of them
were copied from never was. `min-height: 0` on a flex item whose automatic minimum was its content is
the whole fix; §79f's sentence is updated rather than left naming a bug that is gone.

Every reachable state of the screen was re-measured after the change — the safe body at 4, 5, 6, 7,
8, 10, 16 and 44 files a side and with both sides long, the empty plan that routes here, the list
body at 9, 21, 61 and 69 actions and with thirteen gated rows, the checking body, and the failed body
at 30 bytes, 1.4 KB and 10 KB. All of them 764×1040, nothing over the footer.

## The four openers (2026-08-14)

## 97. Two of them cannot open what their label promises

`Open both in an editor` (§74), `Open folder`, `Open on Proton Drive` and `Open the system log`
(G18) were four drawn buttons with no command behind them: the first painted live and inert, the
other three omitted rather than painted dead. #220 and #231 close together, because they are one
capability — a command that hands something to the desktop — behind four labels.

`open_paths`, `open_folder`, `open_remote` and `open_system_log` shell `xdg-open`, the way this
project already shells `systemctl`, `proton-drive`, `secret-tool` and `curl`. No plugin: an opener
plugin would be a dependency, a capability grant in `capabilities/default.json` and a Debian build
rule, for what `std::process::Command` does in twenty lines. The webview therefore gets exactly four
named doors out and no general "open anything" permission.

**`Open on Proton Drive` opens the Drive web app and not the file, and the button is honest about
that rather than absent.** A per-file web URL is `/u/0/<shareId>/folder/<linkId>`; the GUI holds
neither id. `proton_id` is the engine's composed `volumeId~nodeId` — an API identity, not a route
the web app resolves — and no field on the status reply, the index, or the CLI's listing carries a
share. Interpolating the ids we do have would ship a 404 behind a button promising a file. So the
URL is a constant in `commands.rs` and the command takes **no argument at all**, which also removes
the only place a URL could have been injected from the webview.

**`Open the system log` opens a snapshot, because a journal has no path.** The daemon logs through
`tracing` to stderr; the shipped user unit captures that in the journal, which is a binary store with
no filename and no registered handler, and a terminal emulator is not something a Linux desktop
guarantees. So `journalctl --user -u proton-syncd -n 1000` is written to
`$XDG_CACHE_HOME/proton-sync/proton-syncd-log.txt` — `.txt` is load-bearing, `xdg-open` picks a
handler by type — with a header naming the live command. A daemon started outside systemd has no
journal at all (`start_service`'s fallback nulls the child's stderr), and that case returns the
command as an error rather than opening an empty file, which would read as "nothing is wrong".

**Every path is re-validated at the command boundary**, though both conflict paths come off a
`Conflict` the GUI produced itself: `gui_core::opener` refuses absolute paths, `..`, prefix
components — and, by canonicalising both sides, a symlink inside the sync folder that points out of
it, which the textual guard cannot see. A missing file is refused rather than handed over. Nothing
builds a shell string; `Command` takes an argv, so a filename containing `;` or `$(…)` is one
argument.

**A refused or failed open says so, in mono, under the button that failed.** No frame draws this
row — no frame draws a failure — and without it the four buttons fail exactly the way they behaved
before they were wired. `xdg-open` exiting 3 (no handler registered) is the "no editor configured"
case and names the file it could not open. On `6a Details`, the one dialog with a fixed height, the
line goes INSIDE the body rather than under the foot: `.dialog` clips what overflows it, and a
message nobody can see is the bug this whole change is about.

**`Installation help` is still not drawn, and #231 was never what held it.** #244 has since given
the takeover sub-screens with a way back, so the "nowhere to come back from" half is gone too —
what remains is #218: no distribution packages the CLI, and this project's own documentation names
no URL to send anyone to. A button that opened something plausible would be the drawn command box's
own bug one layer up.

## Two facts the index did not record (2026-08-16)

## 98. What cannot be synced is named, and when Proton Drive received it is sourced

G19 (#232) and G20 (#233) are the same shape: one fact about one entity that nothing recorded, so
the screen omitted the clause that needed it. Neither was closed by looking harder at an existing
field — a screen that could have derived either would already have drawn it.

**`Can't be synced` was never a question about the index, and it still is not.** The scanner keeps
only entries where `file_type.is_file()`, so a socket, a symlink, a FIFO or a device node never
enters the index at all. What changed is that `visit_directory` now REPORTS what it drops instead of
skipping it silently, and the daemon merges those reports into the standing `unsyncable` list that
#295 already built — so the dialog's second group is fed by the walk itself, not by a record nothing
writes.

The rejection it splits is the point. One `continue` used to hide two different facts about a path:
a rule the user wrote, and a thing that is not a file. Those are the two groups on this very dialog,
so the rule test now runs first and an excluded socket is reported as neither — filing someone's own
skip rule under "can't be synced at all" would put the answer in the wrong half of the screen they
opened to find it.

**One filter decides the group, and four numbers read it.** The band's count, the band's second
clause, the dialog's title and the dialog's rows are the same number by construction
(`cannotSyncFrom`), which is this screen's own recorded bug shape. Membership lives in
`ACTIVITY.neverSyncedDialog.cannotKind` — under the sentence it has to be true of, because that
table is also the row notes. `remote_not_downloadable` is excluded from it deliberately: a Proton
Docs document is a real file on Proton Drive, and both of this dialog's sentences (`live in your
folder`, `Not real files`) would be false about it. `proton-sync status` lists it under a heading
that claims neither.

**A reason this build does not know is drawn, not dropped.** The wire token is hand-serialized
precisely so a newer daemon can add a kind, so an unfamiliar one renders verbatim as its own note —
the call `proton-sync status` already makes. Both directions are wrong in some way and they are not
symmetric: hiding a file that cannot sync is the failure #295 is about, and showing a remote one
under this heading is the smaller of the two.

**The band renders on either half alone.** `neverSyncedFrom` returned null when no rule matched,
which suppressed the whole band — so a machine with a socket and no exclude rules drew nothing at
all. `neverSyncedSubject` is the pair, and its total is the sum.

**`received 14:32` comes from a transfer that landed, and only from an upload.** `EmblemStatus`
gains `last_transfer` — `{epoch_secs, direction}`, one nested option rather than two parallel fields
that could disagree — read off the history log #308 already writes behind every landed side effect.
No `file_index` column was added: `proton.rs` parses `activeRevision` for `claimedDigests.sha1` and
nothing else, so there is no remote revision time to read, and a new column would have been a second
copy of a table that already had the answer.

Only `up` renders the clause. A `down` row says when THIS computer received bytes, and a conflict
sidecar's fetch is a `down` row filed under the file's own path — either would put the wrong side's
event on the Proton card. There is no fallback to `mtime`, which is the local modification time and
is exactly what S5 refused to label as a remote event; `receivedAtFrom` returns null before it ever
reads the frame's pinned clock literal, so a fixture cannot conjure the clause either.

**The field is absent more often than present, and that is not a gap.** No transfer on record means
one of four ordinary things: nothing ever transferred, the last transfer aged out of the log's
retention (20k rows / 90 days), the file was adopted rather than transferred (`AutoLink` moves no
bytes), or it has moved since — an event row keeps the path the action landed at. The clause is
omitted in all four.

**One row still falls short of its frame, and it is a fact rather than a command.** `7a Never
synced` draws `projects/current → ~/work/q3`, naming the link's TARGET. `UnsyncableItem` carries the
path and the reason; reading the target means resolving a link the engine has decided not to follow,
which is a second question about the same entity. Invisible to the style gate — `.path-name` is
`flex:1`, so its box matches whatever text is in it, and `assert.mjs` does not compare text — so it
is recorded here rather than as a `known-deviations` row that would have nothing to measure.

### 98a. The skip tab's panel, and the 50px that was hiding behind it

`8a Skip rules`' unsyncable panel took the same list. It is a PANEL and not a rule row — `--panel`
over `--border-subtle` at 11px, where a rule row has neither — because there is nothing in it to
edit, which is what its own second sentence says. `See them` opens `7a Never synced`, and the three
surfaces (this count, these kinds, that dialog's rows) read one function rather than three readings
of one list.

`SETTINGS.unsyncableNote` stopped being a constant and became a template, so it landed in the copy
gate's `DRAWN` table in the same commit — the rule S1 wrote the first time a sentence left that gate
by acquiring an argument, and the fourth time it has come up. `cardinal`, not `count`: the sentence
opens with the number and the frame spells it.

**Two `known-deviations` rows went, and one came back as a decision.** The `.sync` note's 12px
margin was recorded as belonging to nothing; it belongs to the panel, and is set with it rather than
by the stylesheet, so with no panel it is still spacing nothing. The tail's `margin-top:auto` was
recorded at `106.812px vs 85.8125px` and blamed on the missing panel — the panel accounts for
exactly 71px of it, and what is left is `35.8125px vs 85.8125px`: the 50px §94's doors take off every
content region. So the row is re-recorded against that decision, which is what it always was.

### 98b. One row that read like the same gap and was not — now closed

**Closed by #315.** The row is gone from §79c and this section is kept for the diagnosis, which is
the part worth having: it took two wrong reasons before the real one.

`9a Review`'s `3 files can't be synced — a socket and two shortcuts` drew without its kinds, and
§79c recorded the reason as "those files never enter the index". They still do not — and that was
never what stopped this row. The number on it was `PlanSummary::skipped_unsupported`, a statistic of
the **dry-run plan**, which counts remote nodes the CLI cannot fetch as bytes. The local kinds are
deliberately kept out of a plan (a socket that replaced a synced file would put two rows for one
path in one plan), and `DryRunReport` deliberately carried no `unsyncable` list: that list is a
_persistent merged_ store, and a one-shot report has no store to merge into, so the same field name
on it would mean something else. Two different sets, unsummable, and on a first run the standing
list is empty anyway because no pass has run.

**What closed it was a third field, not a reconciliation of the two.** `DryRunReport.cannot_sync`
(PR #318) is the plan's own local stat-walk reporting what it dropped — one observation, no age,
nothing merged into it, and named so that it cannot be mistaken for the store. Both halves of the
sentence come off it, so the clause has one source; `skipped_unsupported` is not summed into it and
is not drawn on this screen at all, which is the same verdict S5 already reached from the other side
(`cannotSyncFrom` excludes `remote_not_downloadable` outright — a Proton Docs file is a real file on
Proton Drive, not a non-file in your folder). The count survives where it means something: the
Activity counters, and `actionsThatHappen`, which still subtracts those rows from `See all N
actions`.

The general lesson stands and is why this is kept: a row whose stated reason has quietly become
false is how "nothing enumerates the kinds" gets believed twice. The corrected reason then said the
two facts could not be sourced from one place _today_ — which was true, and was a statement about a
missing producer rather than an impossibility. Naming the producer it lacked is what let the next
change build it.

## The GUI catches up with the daemon it now has (2026-08-16)

## 99. One node no frame draws, and one command with no caller

Three surfaces where the daemon had already published a fact and the GUI was still deciding it for
itself (#311, #315, #319). Two of them are covered above — #315 closes §98b, and #311's half of §79e
is reworded because the function that sentence blamed no longer exists. This section is the third
and the two decisions that have no drawn evidence behind them.

### 99a. `+n more` is drawn by no frame, and that is not a `KNOWN_UNSTAMPED` row

`ControlCommand::PlanResult` bounds its rows (`PLAN_ACTIONS_DEFAULT_LIMIT` 500,
`PLAN_ACTIONS_MAX_LIMIT` 5000). The screen already counted the plan rather than the window —
`summarise` takes `summary.total`, so `The next sync moves 12,480 things` is right over 5,000 rows —
and what was missing was the line saying the list is a window. It is now the last line inside the
scroller, `+7,480 more`, sized from `summary.total - rows.length`.

**None of the three plan frames has a plan long enough to truncate**, so no frame draws this node.
The issue expected a `KNOWN_UNSTAMPED` row and that is the wrong instrument: that file is for a node
**the frame draws and the app cannot**, and every row in it names a `key` that exists in a frame.
This is the opposite — a node the app draws and no frame has. It needs nothing from the fidelity
gate at all: it is not declared in `fids.js` (a slot whose key exists in no declaring frame fails
`check-fixtures.mjs`), it stamps nothing, and no fixture reaches the cap that would render it. It is
covered by `plan.test.js` instead, which is where an undrawn state belongs.

`MAIN.andMore` is borrowed rather than copied. It is the same sentence about the same thing — a
bounded window with a daemon-sized remainder behind it — and `MAIN.authExpiredSub`, quoted by S1
from `11a Outage`, is the precedent for a deck entry outliving its screen name.

### 99b. `list_remote` is deleted, not ported — and the probe that outlived it now asks the daemon

The GUI shelled `proton-drive filesystem list --json` from its own process, outside the daemon's
`CliGate`, against a CLI whose SQLite store is not concurrency-safe (#23). #311 proposed replacing it
with the socket's `list` verb. **Nothing called it** — no `listRemote` call site exists in
`gui/src/js`, and the surface that would want one (`Browse Proton Drive…`, §79e) is unbuilt — so it
is gone rather than rewritten: a socket-backed command with no caller moves "a verb nothing calls"
one layer up instead of removing it. A picker, when it is built, calls `list` through `gui_core::ipc`
like every other verb.

**The one CLI-shelling path that survived this section is closed — #323.** `probe_folder`'s remote
side (`gui_core::folder_probe`, up to `MAX_REMOTE_LISTINGS` = 64 subprocesses) prices a **candidate**
folder — a path outside `remote_root`, before any pair is configured, which was precisely the
question `ControlCommand::List` could not answer: its selector was `remote_root`-relative. That
selector now also accepts an **absolute** one, so the probe walks over the socket, one `list` per
directory, every child spawned by the daemon behind its one `CliGate`.

The walk stayed on the client, which is the decision worth recording: `list` may run on the daemon's
IPC task only because it is _one_ invocation under a bounded gate wait, and the other daemon-side
shape (`Plan`'s ack-plus-latch on the main loop) queues behind whatever pass is running — up to half
an hour for a number a user is waiting on. One request per directory answers in the _gaps_ of a live
pass instead, because the gate is held for one child and not one pass.

`probe_remote_via_cli` remains for the case with no daemon to ask — onboarding, where four of the
five `9a` frames are drawn — under `run_dry_run`'s rule: the child **only** when nothing answers the
socket at all, decided by a follow-up `status` rather than by the failed request.

**Correction to the issue's premise, recorded because it changes what "unbuilt" means here.** #323
says the probe is called where `list_remote` was not — "`9a Folders` is a drawn screen". The screen
is drawn; the call is not made. `probe_folder` has no `gui/src/js` caller, and `onboarding.js` says
so at the site: _"The stats row and the account line are omitted, not blanked: nothing counts the
files or bytes under a candidate folder on either side (#240)"_. So the hazard was latent rather than
live. It was fixed rather than deleted anyway — unlike `list_remote`, #240 is closed with this
capability as its answer, and the command is registered and invokable.

### 99c. One number the `+n more` line made visible, also filed — #324

`summarise` builds one model from two sources: `total` is the daemon's, and `conflicts` /
`uploads` / `downloads` / `newFolders` / `renames` are counted over the **window**. The list head
puts two of them in one sentence, so a truncated plan reads `12,480 actions · 1 conflict kept as
both copies` where the `1` counts only the conflicts that fit. Measured, not reasoned: 5,000 rows
carried out of 12,480 renders exactly that.

Nothing dangerous is understated — destructive rows are never truncated out, so the band, the gate
and `files_at_risk` are unaffected — and the honest fix has two different answers (head counts can
read `summary.*` today; the safe body's side _lists_ cannot, being lists of rows nobody sent). Two
answers for one function is why it is its own issue rather than the tail of this one.

## Local deletions go to the trash (2026-08-27)

## 100. Every deletions frame draws the mode that stopped being the default

`openspec/changes/trash-local-deletes`. A local deletion now goes to this computer's Trash by
default and permanent deletion is a setting. The whole `4a` set — and `12a Deletions light`, and
`8a Deletions tab` — was drawn against a build where a local deletion was _always_ an unlink, so the
drawings and the product now disagree on the default. Nothing in the deck was deleted to achieve
this: every permanent-mode sentence is still drawn, still gated, and still what the screen says once
the user opts back in. What has no drawing is the other mode.

**The frames say permanent, and here is where they say it.** `4a Deletions` and `12a Deletions
light` draw the local card as `Permanent · this computer` over `Removed straight from disk. Not
moved to any trash, and not recoverable from Proton.`; `4a Armed` draws the typed-`DELETE` gate over
`Delete permanently`; `4a Compact` says `1,204 photos gone from this computer, permanently`. All
four are unchanged and still compared.

**Eleven strings are exempt in `copy-gate.mjs`, in two groups.** Five are the recoverable-mode
deletions copy (`recoverableLocal`, `recoverableLocalSub`, `recoverableMixedSub`,
`travelExplainerLocal`, `toTrashLocal`) — no frame draws a local deletion in the recoverable column.
Six are the disposal panel (`disposalTitle`/`Sub`, `disposalTrash`/`Sub`, `disposalPermanent`/`Sub`)
— `8a Deletions tab` draws one panel, the `deletion_policy` guard, and the tab now carries a second
beneath it. The gate reads 342/342 drawn strings matched, 86 exempt, 0 missing; it was 75 exempt
before this change. `recoverableMixed` is _not_ exempt: it is the bare word `Recoverable`, which
`4a Deletions` contains inside `Recoverable · Proton Drive`, so the gate finds it.

### 100a. The second panel is drawn by no frame, and no gate can see that

`assert.mjs` compares a node only when the app stamps it _and_ a frame declares the same key —
"an unstamped node is simply not compared" (assert.mjs:327). The disposal panel is stamped by
nothing, because `8a Deletions tab` has no slot for it. So adding a whole panel to a drawn tab
changed the gate's reading by nothing at all: 51/51 frames, 96793 assertions, 0 failures, before and
after.

This is **#250's gap**, not a new one — a drawn node claimed by no slot has no rule saying whether
it is deliberate, and this panel is now one of them. It is deliberate, and this section is the only
record of that. Coverage is `settings.test.js`'s _"the disposal cards are their own setting,
defaulting to the recoverable one"_, which is where an undrawn state belongs.

### 100b. The mixed queue has no fixture, and could not have one

The change's own task list asked for a fixture whose queue holds a trashed local deletion beside a
Proton-side one — the arrangement that makes the recoverable column's header drop its destination.
**`check-fixtures.mjs` compares the fixture registry against `frames/index.json` in both
directions**, so a fixture label the prototype does not draw is a build failure, not an extra test.
Writing one would have meant inventing an index entry for a frame nobody designed.

It is covered by `deletions.test.js`'s _"a column's header names what is in it, not which column it
is"_, which asserts all four combinations against `columnCopy` directly, including
`columnCopy("recoverable", [file, trashedFile])` → `recoverableMixed`/`recoverableMixedSub`. What is
**not** covered is any rendered check of that arrangement: no pixel gate sees the mixed column. The
honest close for both this and §100a is a designed frame for trash mode, which is design work and
not this change's.

### 100c. `4a`'s fixtures now name the mode they were always assuming

`fixtures/deletions.js` states `disposal: "permanent"` on the local card and `"recoverable"` on the
Proton-side one, rather than letting the field default. The field is absent-tolerant and fails
closed — `severityOfItem` reads a missing, empty or unrecognised `disposal` as `permanent`, so the
frames would have kept matching either way. Stating it is what stops a later change to that default
from silently re-pointing these frames at a mode they do not draw.

`fixtures/settings.js` states `local_delete_mode: "trash"` for the opposite reason: it is the
product default, and the panel it drives is compared against nothing (§100a).

### 100d. Three more sentences were keyed on an identity this change removed

An adversarial review refuted two claims made above, and both corrections are here rather than
edited invisibly into the text that was wrong.

**The claim "every permanent-mode sentence is still drawn, still gated, and still what the screen
says once the user opts back in" was false when written.** It audited the `4a` Deletions frames and
never looked at the Settings tab. `SETTINGS.askPermanentSub` and `askNeverSub` rendered
**unconditionally** — the first promising that "anything removed from this computer for good still
waits for you" when nothing is removed for good, the second saying deletions go "permanently from
this computer" when they go to its Trash. Both sat directly above the new disposal panel saying the
opposite. They are now chosen by `policyCopyFor(disposal)`: the drawn pair under permanent mode,
a truthful pair under trash mode, and the **permanent** pair whenever the mode is unknown or the
config has not been read, because that is the over-warning one.

**Still open, and it needs a design decision rather than a sentence.** The card _title_
`Only ask about permanent ones` is keyed on the same dead identity. Under trash mode nothing is
permanent, yet the setting still guards every deletion applied on this computer — so it reads like
_never ask_ and behaves like _ask about local ones_. Renaming a drawn radio card is design work, and
`deletion_policy`'s semantics were deliberately left untouched by this change.

**Two more labels had the same shape.** `factsOf` was the fourth label still asking severity rather
than direction, so the commonest card in the product read `deleted here 22m ago` above
`You deleted this on Proton Drive` — two sentences on one card, contradicting. And the Plan screen's
typed-`DELETE` gate (#227) asserted `nothing will bring it back` for a deletion that goes to the
trash; the plan wire now carries `ReviewedPlan.local_disposal`, on the plan rather than per row,
because `PlannedAction` is the pure planner's type and disposal is decided at execution time.

**The other refuted claim: "five published pages".** There were six. `safety/delete-approval.md` —
the page the rewritten `safety/deletions.md` links to for exactly this question — still read
_"This is the **permanent** local delete."_

---

## 101. §58d's tie is broken: the tray takes the desktop-facing edge

**Maintainer decision, 2026-08-17 (#261):** `10-tray.md` wins over the four standalone frames.
`.compact-panel.is-tray` now takes `border:1px solid rgba(255,255,255,.1)` (`--compact-tray-border`)
instead of the app's `--border-chrome`, because a panel floating over an arbitrary wallpaper has no
surface behind it to be flush with — the reasoning §58d recorded but did not yet act on, and the one
`10a In situ` supplies since it is the only frame drawn where the panel actually lives. `.is-attention`
keeps overriding it (unchanged from §58d): the crimson edge is the panel saying something is waiting
on you, which stayed the more load-bearing value throughout.

**The bookkeeping does not net to fewer rows, and should not.** Four `10a In situ` border rows
existed under §58d; the code change does not retire them. `10a In situ` is drawn `needsYou`, so
`.is-attention` applies to it precisely as it does in the built app — meaning the frame's frozen
capture (the desktop edge, `rgba(255,255,255,.1)`) and the app's live output (the attention edge,
`rgba(255,107,107,.3)`) are now compared for a **different reason** than before but still disagree,
because the decision's own second clause forbids the one change that would make them match. The row
stays in `known-deviations.mjs`, reworded to say so. What the decision does move is the other four:
`10a Settled`/`Syncing`/`Offline`/`Paused` draw no `needsYou` state, took the tray edge cleanly, and
now disagree with their own (unredrawn) frames, which still show `#23262D`. Four new rows, one per
frame. Net count: five rows citing this decision where one cited §58d — **up**, and that is correct:
a recorded disagreement naming a decision is not a gap the way an unexplained one is.

**Ideally the four standalone frames are redrawn instead**, deleting their new rows in favour of a
prototype that shows the edge the build now draws — raised with whoever holds the design file
alongside the #272/#273 briefs, per the decision. Not done here: `docs/design-v2/Drive Sync.dc.html`
has had exactly one commit since it was added (the import itself), and every other row in this file
resolves a mismatch by changing the app or by recording it — never by hand-editing the frozen
capture to agree with the app after the fact, which is what a same-PR redraw would be.

---

## 102. §72's three options resolve to the first: the command box is dropped, not fixed

**Maintainer decision, 2026-08-17 (#218):** drop the package-manager command; show the manual install
path for every distribution; keep the `Detected …` clause. Of §72's three options — drop the box,
swap its contents for a real command, or ship packages to the repos it implies — the third was ruled
out by its own size and the second by there being no real per-distro command to put in: this
project's own `installation.md` gives one instruction for every distribution, "install it separately,
following its own documentation." A per-distro variant would only have restated the same wrong
promise in five spellings.

**What changed, in `gui/src/js/ui/copy.js`:**

- `CLI_INSTALL_COMMANDS` and `ONBOARDING.cliInstallCommand` are deleted. There is no replacement
  table — nothing this project publishes names a real command for any distribution, so a table with
  fewer wrong entries is still a table of wrong entries.
- `ONBOARDING.cliMissingBody` now reads: _"This app drives the official proton-drive tool rather
  than talking to Proton directly, and no Linux distribution packages it — install it yourself,
  following its own instructions, then sign in and check again."_, followed by `Detected Debian —
that doesn't change what to do here.` (or the undetected form). `Detected …` is unchanged in kind —
  it still says the app recognises the machine — but no longer implies a different instruction
  follows for a different distribution, because none does.
- No URL. #231 (`open_remote`) can point the surface at one, but `installation.md` has been checked
  and names none for `proton-drive` itself — only for this project's own binaries and packages. The
  honest sentence is the one above: go and find Proton's own instructions, not a link this project
  cannot stand behind.

**What did not change:** `renderCliMissing` (`gui/src/js/screens/onboarding.js`) already shipped no
copyable command box and no `Installation help` control — C5 built the `Detected …` half and left the
rest for this decision, so #218's "S7 must not ship one" was never violated and there is nothing to
un-ship. `ONBOARDING.copy` (`"Copy"`) and `.installHelp` (`"Installation help"`) are untouched: the
frozen `9a CLI missing` frame still draws both, on a command box and a help button neither exists in
the build, and the deck's job — same as `cliInstallCommand`'s until this decision — is to say so
truthfully, not to render them.

**The bookkeeping, in `gui/tools/fidelity/`:**

- `copy-gate.mjs`'s `DRAWN` rows for `ONBOARDING.cliInstallCommand` and `.cliMissingBody` are both
  removed. The first because the function is gone; the second because its new sentence is not a
  substring of the frozen frame's text the way the old one was (the frame still says "other
  distributions are in the help", which is no longer true and is not repeated). Confirmed by running
  the gate rather than reasoning about it: `fidelity:copy` reads 340/340 after the change, down from
  342/342 by exactly the two retired checks, 0 missing.
- `known-deviations.mjs`'s two existing `9a CLI missing · box.h` rows (`div`, `div/div`) moved from
  `176 vs 116` to `176 vs 136` — the manual-path sentence is longer than the two it replaced, so the
  20px gap it used to leave narrowed rather than closed. A third row is new: `div/div/div[1]` (the
  body paragraph itself) now reads `40 vs 60`, where before the change its height matched the frame's
  within the gate's 0.5px tolerance and needed no row at all. All three numbers are read off a real
  `npm run fidelity` run, not computed by hand, per this file's own rule.
- All three `9a CLI missing` rows, and all five tray-border rows in §101, changed from `issue` to
  `decision`.
  `known-deviations.mjs`'s own schema is explicit that `issue` means "the issue that closes it" —
  #218 closes on this same PR, so a row still citing it would name an issue that can never close it
  again. `decision: true` is the tag written for exactly this: the product chose against
  the drawing, permanently, and DEVIATIONS carries the decision and its date instead.

**Confirmed, not assumed:** `npm run fidelity` reads 51/51 frames mapped, 96793 assertions, 0
failures, no stale deviations, after every change above.

---

## 103. The shell does not resize for the cleared state — a decision taken, reversed, and then the bookkeeping for the reversal

**Maintainer decision, 2026-08-17 (#221), in two parts 3m10s apart (00:19:57Z → 00:23:07Z).** The first comment chose
option 1 — shrink the window to 522 for `3a Conflicts cleared` and grow it back on the way out — and
recorded it as the opposite of the reviewer's own recommendation. The second, headed **"Correction —
superseding the previous comment"**, reverses it: **do not resize; keep the shell as it is.** Nothing
had been implemented against the first comment, so there was no code to unwind — only the record of a
decision taken and then changed. This section documents the second, settled answer. A reader reaching
this file from the issue thread and stopping at the first decision comment would build the wrong
thing; the correction is what governs.

**What this makes true.** `tauri::Window::set_size` is not called on this state, and `02-shell.md`
gains a property it did not previously state: the window is never resized by the app itself, joining
`DECISIONS.md` #2's existing "fixed at 1040×764" as something the shell doc says in its own words
rather than something a reader has to reconstruct from an issue thread. **Note the premise this
corrects, since the record is the point:** the issue body and both decision comments assert that
`02-shell.md` already says the window "never moves" — checked against the file, it does not; nothing
there speaks to window position or size at all (only to the hexagon and the four doors, which never
move on their own axis). That claim was invented once and repeated twice without being checked. The
522px drawing is read as a frame made in isolation, not as an instruction that the app changes its own
geometry mid-session — the cleared body renders as a centred 520px column inside the fixed 1040×764
shell, and the footer stays window-width, exactly as it already did. **This does not touch
`resizable: false`.**
User-driven resizing (#273) stays deferred behind re-drawn frames; app-driven sizing for one state and
user-driven sizing for every state are two different capabilities, and this decision was the one
place they could have been conflated by accident. They do not interact: nothing here grants the user
a drag handle, and #273 is neither closed nor advanced by it.

**S3's empty state (`4a Empty`) has its precedent now, and it costs nothing.** The triage blocker that
held this issue back explicitly named it — "whichever answer is given must also cover `4a Empty`" —
and the correction answers it in one clause: _"settle at the shell's size, do not shrink."_ `4a Empty`
already does, because nothing has ever resized the shell for it either; there is no S3 code to change.

**The bookkeeping goes one frame further than the decision's own paragraph names, and that is flagged
here rather than done silently.** The decision's own "The bookkeeping, which is the only work left"
paragraph — not its "what this makes true" section, which is three bullets about `set_size`, #273 and
the S3 precedent and never mentions the rows at all — names the 15 `3a Conflicts cleared` rows in
`known-deviations.mjs`. Grepping the file for `#221` at the
time this section was written found **26**: the same 15, plus 3 on `4a Empty` (§75) and 8 on
`5a Checking` (§76/§77) — the plan screen's own narrow-window state, which no comment in the issue
thread ever mentions. All three frames cite `#221` for the identical cause (the shell's fixed 1040
against a frame drawn as its own 522/520 window), and this PR closes #221. `known-deviations.mjs`'s
own schema is explicit that `issue` means "the issue that closes it" — a row still citing #221 after
this PR would name an issue that can never close it again, which is precisely the self-invalidation
the decision's closing lines warn against. So **all 26** rows move from `issue: "#221"` to
`decision: true`, not just the 15 the paragraph enumerates; §75 and §76/§77 are amended below to match.
If the maintainer meant to leave `4a Empty` or `5a Checking` open pending a redraw rather than settled
by the same reasoning, that is the one point in this section to veto — nothing about the code changed
either way, only the citation.

**§74's `3a Conflicts cleared` paragraph, §75's `4a Empty` paragraph, and §76/§77's `5a Checking`
mentions are pointed here.** Their summary-table `gap` cells (§74's `3a Conflicts cleared` row, §75's
`4a Empty` row, §76's `5a Checking` row) change from `#221` to `decision`, matching the existing
`4a Deletions`/`4a Armed` rows in §75's own table.

**`02-shell.md` gains one line** stating the window never resizes itself, alongside the existing
never-moves property, so the next screen that asks this question finds the answer written down rather
than re-litigating it from an issue thread.

**The harness could not have told the two options apart, and that is worth recording for whoever next
reaches for `tauri::Window::set_size`.** `gui/tools/fidelity/assert.mjs` renders every frame in
headless Chromium at a fixed **width** of 1040 (`page.setViewport({ width: 1040, height: 764 })`,
line 85) — there is no real Tauri window anywhere in this harness. (The height alone varies once,
dropping to 200 for the unrelated squeeze gate at line 549, which re-renders the compact tray frames
in less room than the panel wants; the width this decision turns on never does, for any frame.) Had
option 1 shipped, the gate would have gone on rendering the cleared body inside a 1040-wide page
exactly as it does today; nothing would have moved. The rows still resolving under `decision: true`
rather than
disappearing is what keeps the frame _compared_ either way (assert.mjs's `unmetDeviations` still fails
the build the day one of these 26 stops mismatching) — the requirement the decision's closing
paragraph names, satisfied by the same mechanism §101 and §102 used, not by deleting rows the gate
still needs.

**The bookkeeping, in `gui/tools/fidelity/known-deviations.mjs`:**

- All 15 `3a Conflicts cleared` rows, all 3 `4a Empty` rows, and all 8 `5a Checking` rows — 26 in
  total — changed from `issue: "#221"` to `decision: true`, each `why` rewritten to the
  `MAINTAINER DECISION, 2026-08-17 (#221): … DEVIATIONS §103` template §101 and §102 established.
  `detail` is untouched on every row: the schema requires it verbatim as `assert.mjs` formats it, and
  nothing about the drawn output changed.
- The S4 section header comment, which had read "sixteen" for the `5a Checking` rows since before the
  eight door-node rows were retired with the doors themselves (§77), is corrected to "eight" in the
  same edit — a pre-existing staleness this pass could not leave untouched while rewriting the section
  it sits in.

**Confirmed, not assumed:** `npm run fidelity` on the unmodified branch reads 51/51 frames mapped,
96793 assertions, 0 failures, 67 Phase-1 deviations, 5 structural, 45 decided. After the 26-row
conversion above: 51/51 frames mapped, 96793 assertions, 0 failures, 41 Phase-1 deviations, 5
structural, **71 decided** — the assertion count unchanged (nothing rendered differently), the
Phase-1/decided split moved by exactly 26 in opposite directions, and no stale-deviation error either
run.

## 104. The full-sweep schedule (G4, #193) — built, and the two places the drawing contradicts itself

`full_scan_schedule` is now a config key, a daemon timer and the panel `8a Settings` draws. This
section records what building it settled, what it retired, and the two states the frames do not
cover — the second of which is a drawn sentence that is **false about its own drawn value**.

**The three `#193` deviation rows are retired, and the replacement is declared rather than merely
present.** The rows recorded the schedule panel's head-row text block, its title and its sub-line
each taking `938px` where the frame draws `762.88` — the width the missing Weekly/Monthly control
and its 20px gap were holding. `scheduleMode` (and the two segment buttons) are now in `fids.js`, so
those three nodes are checked rather than un-flagged. **Verified by counterfactual**, because a
retirement and a fresh divergence cancel out silently: with the control removed again,
`npm run fidelity` reports `3 failures` on exactly those three keys plus `3 unexplained unstamped
slot(s)`. Restored: `51/51 frames mapped, 97896 assertions, 0 failures`.

### §104a · No schedule is a state, and no frame draws it

Every config written before this key has none, which is most of them. The controls have to rest
somewhere, so they rest where the frame draws them — `Weekly`, `Sun` unselected, `03:00` — but the
key line reads `full_scan_schedule · not set` and a sentence beneath says *No full sweep is
scheduled. Pick a day to start one.*

Rendering `full_scan_schedule · weekly sun 03:00` under untouched controls was the alternative, and
it is exactly #347's defect on the screen whose entire job is to report what the file says: a
sentence the app cannot have computed, drawn as if it were live. Choosing a day is what commits;
switching Weekly/Monthly commits nothing, because a mode is not a schedule and converting one would
move a sweep to a day nobody chose.

Both strings are exempt in `copy-gate.mjs` with that reason.

### §104b · The monthly grid cannot reach the days its own note is about

`8a Schedule monthly` draws a `repeat(10,1fr)` day grid whose box is **56px — two complete rows, 20
chips**, not a clipped view of more. Beneath it: *Months without a 15th are skipped to the last day.*

Two things follow, and they are both measured rather than argued:

1. **Every month has a 15th**, so the drawn sentence is false about the drawn selection (`15`, the
   one chip the frame paints selected).
2. **Every month has 20 days**, so the sentence cannot become true for *any* value the drawn grid
   offers. It is plainly a template with its day interpolated, and read as one it is exactly right.

The maintainer decision (2026-08-17) names the fallback rule as part of what to build — *"Monthly
carries the drawn rule that months without the chosen day fall back to the last day"* — and that
rule is unreachable at 20 chips. So the grid is **1..31** and the note is
`SETTINGS.monthEdgeNote(day)`, shown only for **29, 30 and 31**: the three days a month can actually
lack. `src/schedule.rs` implements the clamp as `min(day, last day of that month)` and pins it for
February in both a common and a leap year.

The monthly crop maps only its header (§ the two frames disagree about the head row's `gap`, 18px
against 20px), so none of this is gate-checked there — which is the reason to write the measurement
down here rather than leave it in a commit message.

### §104c · The sub-line loses its count, and it is G7's gap rather than G4's

The drawn sub-line is *A full check of all 12,480 files as a safety net…*. The only thing that
counts the sync folder's files is `skip_rule_usage`, a full metadata walk the Folders tab
deliberately does not run — the same folder-totals gap (#207/G7) that already costs the local
helper one panel up its first sentence. `SETTINGS.fullScanSubUnknown` drops the clause and keeps the
rest.

Explicitly **not** "use the count when the Skip tab happens to have loaded it": the sentence would
then gain and lose a number depending on which tab was visited last, which is worse than a stable
omission and is precisely the kind of thing no gate would catch.

### §104d · What was deleted rather than exempted

`SETTINGS.timer` / `.timerSub` (the Phase-1 panel's own honest title, *Look for changes on a
timer*), `.timerUnit`, `.timerSeconds`, `intervalLabel`, `stepInterval`, `MIN_INTERVAL_SECS`,
`MAX_INTERVAL_SECS` and the `onInterval` handler are all gone. Nothing renders them, and a string no
screen draws is one the next reader has to establish the status of.

`scan_interval_secs` **is not gone from the config**: it still governs the ordinary pass cadence in
degraded/snapshot mode. What the design removes is its presence in Settings as a user-facing
full-sweep dial, which is what it had become for want of anything else to put there.

### §104e · One further stale comment, found on the way

`ui/controls.js`'s `quiet` button kind was documented as *"the `⋯` and the segmented control's
unselected segments"*. The frame draws that segment at `--text-3` (`#99A2AE`), a step brighter than
`quiet`'s `--text-label` (`#626B78`) — so the claim was wrong, and could not be wrong *visibly* while
no screen drew a segmented control. A `segment` kind carries the correct tone; `quiet` keeps the `⋯`.

Two CSS comments in the same file said the day-chip and stepper numbers were "not numbers any gate
checks", true only while nothing rendered them. Both corrected.
