# Deviations

Where `docs/design-v2` and the prototype disagree, and what was done about it. Resolutions follow
the precedence rule in `IMPLEMENTATION-PLAN.md` §1.3:

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
Each is a guess constrained by the surrounding ramp. **S10 must verify or replace them, and P0.2
should ask the designer:**

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

| #   | what is missing                                                                                        | frames                                                                                                                 | issue                                                                   |
| --- | ------------------------------------------------------------------------------------------------------ | ---------------------------------------------------------------------------------------------------------------------- | ----------------------------------------------------------------------- |
| G6  | a **size on a planned action** — `PlannedAction` has no size field at any level of the dry-run surface | `5a Plan` (`3 files, 4.1 MB`), `5a Plan safe` (a size per row), `9a Review` (`1.4 GB` / `38.4 GB`)                     | [#206](https://github.com/osirison/proton-drive-sync-engine/issues/206) |
| G7  | **index-wide totals** — how many files, how many bytes                                                 | `2a Settled`, `2a Compact settled`, `10a Settled`, `7a Activity quiet`, `8a Settings`, `5a Checking`                   | [#207](https://github.com/osirison/proton-drive-sync-engine/issues/207) |
| G8  | a **subtree aggregate** for a directory about to be deleted, and an atime                              | `4a Deletions` (`1,204 photos, 8.4 GB`, `last opened Mar 2024`), `4a Armed` (the same count in the confirmation title) | [#208](https://github.com/osirison/proton-drive-sync-engine/issues/208) |
| G9  | **dry-run progress** — `run_dry_run` resolves once and reports nothing while it runs                   | `5a Checking` (`8,431 of 12,480 files`)                                                                                | [#209](https://github.com/osirison/proton-drive-sync-engine/issues/209) |

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

| drawn                                                    | frame        | what exists                                                    | issue                                                                       |
| -------------------------------------------------------- | ------------ | -------------------------------------------------------------- | --------------------------------------------------------------------------- |
| `· 12,480 files · 41.2 GB` on the settled sub-line       | `2a Settled` | `last_sync_epoch_secs` and nothing else — no index-wide totals | G7 [#207](https://github.com/osirison/proton-drive-sync-engine/issues/207)  |
| three transfer rows, two directions, a `queued` one      | `2a Syncing` | `activity.transfer`, a single `Option<TransferActivity>`       | G10 [#211](https://github.com/osirison/proton-drive-sync-engine/issues/211) |
| a 2px progress bar at the real percentage                | `2a Syncing` | see below — no percentage is computable at all                 | E1 [#98](https://github.com/osirison/proton-drive-sync-engine/issues/98)    |
| `386 MB sent · 1.1 GB received today` in the footer line | `2a Syncing` | nothing; the shell draws the folder pair instead               | G2 [#191](https://github.com/osirison/proton-drive-sync-engine/issues/191)  |

**The progress bar is unreachable by construction, not merely unimplemented**, and that is sharper
than #98 states it. `TransferActivity` carries `bytes_total` and `bytes_done` and **never both on the
same transfer**: an upload gets `bytes_total` from the local file's size and no `bytes_done` (the CLI
reports none), a download gets `bytes_done` sampled live from the staging directory and no
`bytes_total` (a remote listing carries no size). Neither direction can produce a fraction. So
`transferRow` takes `progress: null` meaning _no track_, distinct from `0`, which would read as
stalled — `rows.js` already carried that distinction for queued rows and it now has a second caller.

**The size chip is drawn on uploads and omitted on downloads** for the same reason, rather than
em-dashed: an em-dash means UNKNOWN in this design, and the daemon was never asked.

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

- **`SyncActivity.since_epoch_secs` is the PHASE's start**, reset by `begin_activity` on every phase
  change. A pass walking scanning → listing → executing → committing therefore counts up and jumps
  back to zero three times. `last_sync_epoch_secs` is the previous pass's _end_, so it is not the
  fallback either. Phase 1 renders the phase's elapsed time — visibly wrong on a long pass, and the
  closest honest number there is.
- **`pending_changes` is the local watch queue and nothing else**, cleared after each successful
  reconcile. A pass driven entirely by Proton — a second device uploading, the first reconcile after
  a restart — has an empty queue while downloading, so the headline read `Syncing 0 changes` with a
  literal `0` inside the mark. Phase 1 takes `last_plan_summary.uploads + downloads` while syncing
  and falls back to the queue: both are the daemon's own numbers, deletions are excluded because
  "the count in the hexagon is transfers, not decisions", and on both drawn frames they agree at 3.
- **`last_plan_summary` is null until `execute_plan_and_commit` runs** — the whole scan-and-walk
  stretch, during which `syncing` is already true. `0 leaving, 0 arriving` is a summary the daemon
  never published, so the clause drops instead. Same rule as §63's omissions and `unreachableBody`'s.

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
| S1     | `2a Syncing`     | three transfer rows, one queued        | the one in-flight transfer          | G10 #211  |
| S1     | `2a Syncing`     | a progress bar at the real percentage  | no track at all                     | E1 #98    |
| S1     | `2a Syncing`     | `386 MB sent · 1.1 GB received today`  | the folder pair (the shell's line)  | G2 #191   |
| S7     | `9a Review`      | `Needs 38.4 GB free. You have 214 GB.` | the free-space half only (§71)      | G6 #206   |
| S2     | `3a Conflict`    | `You added a line, 5 minutes ago`      | the relative time (§70)             | G12 #217  |
| S7     | `9a CLI missing` | `sudo apt install proton-drive`        | the tarball path for everyone (§72) | #218      |

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
| S2     | `3a Conflicts cleared` | a 522px window                    | a 520px column in a 1040 window | #221     |

### 75. S3 · the deletions screen

**`Keep it` has no command behind it, and it is the safe half of the screen.** `ControlCommand::Deny`
is documented as _"Revoke a prior approval (before it has applied)"_ and does exactly that — it
deletes a row from `delete_approvals`. There is no _refusal_: withholding is already the default, so
denying something nobody approved is a no-op. Two things the button's own sentence promises therefore
do not happen. The refusal is not durable (the planner re-derives the same withheld action next pass,
so the row is back on the next status reply and back on the screen at the next launch), and the other
side is not restored (`put it back on Proton Drive` never uploads anything). One engine primitive
covers both directions — purge the baseline `file_index` record so the surviving side stops looking
like a delete and starts looking like a fresh copy — and that is #224. Phase 1 sends `deny`, which is
right in the one case where it does something, and remembers the decision in `deletionsDecided` for as
long as the app is open so the queue can still reach its empty state.

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

**Two numbers and an atime, all #208.** The folder card's `1,204 photos, 8.4 GB` and the armed title's
`1,204 photos` are one subtree aggregate that no command produces; `last opened Mar 2024` is an atime
and the index stores mtime only. Four assertions are recorded, and the shapes differ on purpose:

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

**`deleted on Proton 22m ago` is the age of the PASS, not of the deletion.** `detected_epoch_secs`
reads like when the deletion happened; `decide_delete_gate` stamps `now` on every withheld action,
`self.pending_deletions` is replaced wholesale at the end of every plan, and the incremental
fast-path explicitly cannot idle-skip while anything is pending — so a deletion that happened three
days ago reports an age of seconds, refreshed every ~30s. Omitted, and filed as #225. It takes the
folder card's whole facts strip with it: a folder's other fact is the atime (#208), so there is
nothing left to draw. A file keeps `last edited <month>` from `path_sync_status`.

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

**`DELETIONS.compact.permanent` still hardcodes the aggregate.** `1,204 photos gone from this
computer, permanently` is a fixed string in the deck and drawn in `4a Compact`, so the copy gate is
happy and the panel reproduces the frame. It is the same #208 number, and the day the tray panel
renders a live queue (S8) it needs the same treatment the card got here.

| screen | frame          | drawn                                         | Phase 1                                  | gap      |
| ------ | -------------- | --------------------------------------------- | ---------------------------------------- | -------- |
| S3     | `4a Deletions` | `Deleting this removes 1,204 photos, 8.4 GB…` | the sentence without a figure            | G8 #208  |
| S3     | `4a Deletions` | `last opened Mar 2024`                        | the clause omitted                       | G8 #208  |
| S3     | `4a Deletions` | `deleted on Proton 22m ago`                   | the clause omitted                       | G14 #225 |
| S3     | `4a Deletions` | `Keep it — put it back on Proton Drive`       | `deny`, remembered for this session      | G13 #224 |
| S3     | `4a Armed`     | `Delete 1,204 photos from this computer?`     | `Delete photos/2019 from this computer?` | G8 #208  |
| S3     | `4a Armed`     | `Everything in photos/2019 — 8.4 GB —…`       | the sentence without the size clause     | G8 #208  |
| S3     | `4a Empty`     | a 522px window                                | a 520px column in a 1040 window          | #221     |

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

**The typed word authorises nothing, and that is #227 rather than a shortcut.** `apply_approval_command`
matches against the daemon's **current** `pending_deletions`, and at plan time nothing is pending: a
deletion becomes pending only when a pass withholds it, and no pass has reached this plan. So
`approve(path)` before `sync_now` records nothing. `Run this sync` therefore asks for a sync and
nothing more.

**What happens next is the daemon's own guard, and it is not one answer.** With delete approval on —
the default, both directions — the pass withholds the delete exactly as it would have anyway and it
arrives on the Deletions screen to be answered there: agreed to **twice**, which is safe and is not
what the design says. With it OFF (`--no-delete-approval`, `[delete_approval] remote/local = false`,
or a `.proton-sync.toml` turning the guard off for that subtree — `decide_delete_gate` asks the
per-path resolver first) the pass deletes, and the word typed on this screen is the only thing that
stood in front of it. So the gate is either a formality or the whole protection depending on a config
key this screen never reads, which is the sharpest way to say why #227 matters.

**Both filtered-apply buttons are hidden, not drawn-and-inert.** `Run it without the deletion` is G3
(#192) and `06-plan.md` says outright to hide it rather than fake it. `Leave it alone` — the band's
own escape hatch — is the same capability reached from the band: drop this one action and run the
rest. Read the other way (a durable refusal of this deletion) it is #224, which does not exist
either. So it is hidden by the same rule, and deliberately not left drawn: a button that quietly does
nothing would be worst of all right there, because it is the escape hatch on the one screen where
somebody is looking for one. Four style-gate rows record the widths that change (`#192`).

**No byte total exists anywhere in the dry-run surface.** `PlannedAction` carries `path`,
`destination_path`, `action`, `entity_kind`, `conflict_path` and `remote_id` — and no size. So
`files, 4.1 MB` draws as `files` on both sides of both 1040 frames (four `box.w` rows, G2 #191), and
every per-file size on the safe screen (`1.2 MB`, `2.8 MB`, `96 KB`, `2.4 MB`, `184 KB`) is omitted.
Those cost no assertion at all, which is worth saying rather than leaving to look like coverage:
every one of those rows sits inside a subtree containing an unbundled glyph, so the harness does not
compare their boxes. `new folder` and `moved` are drawable and are drawn.

**`8,431 of 12,480 files` is omitted whole, because its two halves are two different gaps.**
`run_dry_run` is a single async command with no progress channel (G9 #209) and nothing reports an
index-wide file count (G7 #207). Half a fraction is a fraction with no denominator. The `Stop` button
keeps its own 22px margin rather than widening to stand in for the missing line.

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

| screen | frame          | drawn                                   | Phase 1                                 | gap              |
| ------ | -------------- | --------------------------------------- | --------------------------------------- | ---------------- |
| S4     | `5a Plan`      | `3` `files, 4.1 MB`                     | `3` `files`                             | G2 #191          |
| S4     | `5a Plan safe` | a size on every row                     | the row without its size                | G2 #191          |
| S4     | `5a Plan`      | `Run it without the deletion`           | hidden                                  | G3 #192          |
| S4     | `5a Plan`      | `Leave it alone`                        | hidden                                  | G3 #192          |
| S4     | `5a Plan`      | typing `DELETE` authorises the deletion | asked again by S3, or applied unguarded | #227             |
| S4     | `5a Checking`  | `8,431 of 12,480 files`                 | the line omitted                        | G9 #209, G7 #207 |
| S4     | `5a Checking`  | a 522px window                          | a 520px column in a 1040 window         | #221             |
