# Deviations

Where `docs/design-v2` and the prototype disagree, and what was done about it. Resolutions follow
the precedence rule in `IMPLEMENTATION-PLAN.md` §1.3:

1. the `.md` files are normative for **tokens, rules, semantics and copy**;
2. the matching 1040 / 600 / 520 / 360 frame is normative for that screen's **layout geometry and
   per-element colour**;
3. the illustrative swatches in the prototype's "The system" header block are **not** normative.

**Status: partial.** Sections are numbered in the order they were resolved and grouped by the task
that resolved them: **§8–§19 F1** (#165, `tokens.css`), **§20–§31 F2** (#166, the hexagon),
**§32–§39 F3** (#167, the seam), **§40–§46 F4** (#168, the shell). Of conflicts 1–7 in `IMPLEMENTATION-PLAN.md` §1.3, **1** reaches a
token (§18) and **2, 3, 5 and 6** are per-component colour or geometry that F2 and F3 have now
measured; 4 and 7 belong to their screens. The full sweep is **P0.2** (#163).

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

Two sizes are drawn at two widths by *family*: 72px is 4.5 in the `2a`/`12a` compacts but 4.6 in the
`10a` tray panels, and 34px is 6 in-window but 7 in every `11a` notification. The 72px split
contradicts §1.2's "shared component; the tray reuses it" — **F2 and F6 need a joint designer call**.

## 22. "The mark reads as the same weight at every size" is not what is drawn

§6. Rendered stroke falls monotonically from 4.76px at 168 to 1.05px at 14, while *relative* weight
rises from 2.8% to 10% of the mark. That is an optical-compensation ramp, and a good one — but it is
the opposite of constant, so the sentence cannot be implemented as written.

## 23. The perimeter is 303.0115, not ~297 — and the dash arrays are not tuned to it

§6 says *"Perimeter = 297 units - the number the dash arrays are tuned against."* 297 is
`6 x 49.5275`, which assumes a **regular** hexagon. This one is not: the two vertical sides are 52.5
units, the four slants 49.5275 and 49.4783. The true perimeter is **303.0115** (verified twice,
independently).

The dash arrays are tuned against something else entirely: `62+238`, `40+260` and `70+230` all sum
to **300**, which is exactly the `stroke-dashoffset` travel in `hexup`/`hexdn`. *Dash period equals
offset travel* is what makes the loop seamless. The 3.0115-unit remainder leaves a permanent stub of
"on" at the path start (the top vertex) — visible in every drawn frame. **Do not retune**: matching
the true perimeter would remove the stub and break the F8 gate against the frames. Doc error, code
unchanged.

## 24. There is no crimson hero — `2a Needs you` draws the *syncing* mark

The most likely thing to build wrong. At 168px, `2a Needs you` is byte-identical to `2a Syncing`
apart from its gradient ids: same `#191C21` track, same two travelling segments, same neutral
`#F2F4F7` numeral. Not a bug — `03-main-screen.md` says *"the count in the hexagon is transfers, not
decisions"*, and the attention band carries the decisions. **The crimson mark exists only at
<=72px.** §6's five-state table reads as though every state has a hero form; it does not.

## 25. The seam mask is orthogonal to state, and the plan is wrong in both directions

IMPLEMENTATION-PLAN §5's F2 row says *"syncing track carries `fill:<surface>`"*. Measured, `fill`
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
  §6 writes it inside the *Track* cell, which reads as a path property; applying it there leaves the
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
*glow* — a **sibling** 480px div behind a 168px mark, present on only 2 of 9 in-scope settled
hexagons. A component that emitted it would paint 156px outside its own box and fail the fit gate,
so F2 ships the keyframe and S1 places the div.

## 30. Reduced motion: rule 1 governs, and one case needs a designer

The prototype contains **zero** `prefers-reduced-motion` rules, so no frame exists and rule 2 cannot
apply. §7's wording is normative: *"drop the travelling segments to a static 40%-opacity coloured
outline"* — which means the **dasharray goes too**, since a frozen dash is a segment, not an outline.
Two consequences worth stating:

- **The paused dasharray must survive.** It is static geometry carrying the state's meaning, not an
  animation; a blanket "strip dasharray under reduced motion" rule destroys the state.
- **The glow must be pinned to `.45`, not merely un-animated.** Removing the animation returns it to
  `opacity:1` — brighter than the keyframe's own `.8` peak, so honouring the preference would make
  it *more* prominent.

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

§5 rule 1: the seam is drawn *"when data is moving, or when a decision has two sides"*. Taken as a
predicate that would put a seam on `2a Compact needs you`, `12a Compact needs light`, `4a Compact`
and `3a Conflicts cleared` — every one a two-sided decision, none of which draws one. 32 in-scope
frames have no seam. §25's finding points the same way from the other side: `7a File pending` and
`3a Conflict diff` carry the hexagon's seam mask with no seam element anywhere in the frame, so a
masked mark does not imply a seam either.

The rule is right about *intent* and cannot be mechanised. `auditSeams()` therefore checks rules 2
and 3 and deliberately does not check rule 1 — a rule-1 check would report four of the design's own
screens as violations. S1–S11 should add a seam where `SEAM_SITES` has a row and nowhere else.

## 33. It does touch an edge — there are three gradient shapes, not one

§5: *"It fades in and out at both ends against the surface colour — it never touches an edge."* Six
of the 20 run at **full colour** into one end:

| Shape | Sites | Form |
| --- | --- | --- |
| both ends fade | 10 sites, 14 drawn | `S, L a%, L b%, S` |
| bottom cut | `5a Plan`, `5a Plan safe` (hero), `7a Activity quiet`, `7a File lookup`, `9a Review` | `S, L a%, L 100%` |
| top cut | `5a Plan safe` (list) | `L, L b%, S` |

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
full-width in the sense `auditSeams()` tests for. Rule 2's "spans the window" is about *reading* as
a band, and an inset card with a crimson border reads as one. The audit will not catch a seam run
into it; the site rows will.

## 34. The stops are not a function of height, and the fade is a quarter, not an eighth

§5: *"The percentage stops vary by block height (10–30% in, 70–90% out); pick stops that put full
opacity across the content and fade over roughly the top and bottom eighth."* Three claims; the
envelope is right, the other two are not.

**Not a function of height** — two clean disproofs, both between same-shape symmetric seams:

| | height | stops | fade-in |
| --- | --- | --- | --- |
| `2a Needs you` | 508px | 26/78 | 132.1px |
| `4a Deletions` | 514px | 10/90 | 51.4px |
| `5a Checking` | 543px | 30/70 | 162.9px |
| `2a Syncing` | 544px | 26/74 | 141.4px |

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
*measurement*, not about `tokens.css`:

| | dark | light |
| --- | --- | --- |
| `--seam` | `#2A2E36` | `#D9D5CE` |
| `--border` | `#23262D` | `#E6E3DE` |
| **the panel seam, as drawn** | `#23262D` | **`#D9D5CE`** |

`var(--border)` is right in dark and wrong in light; `var(--seam)` is the reverse. The two seam
colours part in dark and meet again in light, so F3 adds `--seam-panel` (`#23262d` / `#d9d5ce`).
This is the §8 pattern with the themes swapped: there, tokens sharing a dark value diverge in light.

Three sites use it: the 360px compact panel, the tray panel, and — the surprise — `5a Checking`, a
522×766 *window*. The 602×542 `9a First sync` window keeps `var(--seam)`, so this is not a
width rule. All three also happen to be the only 30/70 seams in the design; recorded as an
observation, not a rule, on four samples.

## 36. Rule 3 is missing its load-bearing half: the mask must be POSITIONED

§5 rule 3: *"Centred text and centred buttons that sit on the seam get `background:<surface>` plus
`padding:0 14–18px` so the line passes behind them. `z-index` alone is not enough — the line would
still show between glyphs."* The warning is about the wrong hazard. The seam is
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
strength, and `--decision-bg` is `rgba(255, 107, 107, .05)`, a real token that hides nothing. The
subject of the check is anything *claiming* to mask — an element with a background of any opacity
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
- **Reduced motion drops the transition, never the seam.** Rule 1 makes *presence* carry meaning, so
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
52px header is a number you can read out of the source, but *"does this screen have a footer nav"* is
a fact about the tree, and the padding that separates four otherwise-identical footers is layout.

The shell is verified rather than eyeballed: `gui/src/index.html` is loaded in a browser on the
mock-data path and the chrome it builds is asserted against these numbers — 27 checks covering the
header, the chip, the doors, the door/action-bar switch and the home affordance.

## 40. The four doors are not on every screen

`02-shell.md` §"Footer navigation": *"These four never move and never change order, on any screen, in
any state."* True about order, and it reads as *present everywhere*. Measured, every in-scope 1040
frame carries **either** the four doors **or** a footer action bar — never both, never neither:

| | Frames |
| --- | --- |
| four doors (13) | `2a` ×3 · `3a` ×2 · `4a` ×2 · `6a Activity passes` · `7a` ×2 · `12a` ×4 |
| footer action bar (6) | `5a Plan` · `5a Plan safe` · `8a Settings` · `8a Skip rules` · `9a Folders` · `9a Review` |

The action bar **replaces** the doors on the screens that commit something. That is coherent — you do
not wander off mid-commit — but it has a consequence the docs do not draw: **Settings, Plan and
onboarding have no navigation at all.** The only ways out are the action bar's own secondary button
and the app mark.

Which settles half of `IMPLEMENTATION-PLAN.md` §3.3's open question by elimination. The plan
*assumes* "clicking the active door returns to root, and the app mark is also a home affordance"; for
Activity the first works, but on Settings and Plan there is no door to click, so **the app mark is
not optional**. `routes.js` carries a `footer` field per route for this, and `renderHeader` takes an
`onHome`.

## 41. The footer nav has four padding variants, not a range (§1.3 conflict 7, closed)

`0 40px 18–22px` with `padding-top:14–20px`. All four combinations are drawn, so — like the seam's
`-114`/`-150` (§33a) — it is a table, not a range to pick from:

| bottom / top | Mono line | Height | Frames |
| --- | --- | --- | --- |
| 22 / 20 | yes | 89 | `2a Settled` · `2a Syncing` + both light twins |
| 20 / 16 | no | 53 | `2a Needs you` |
| 18 / 15 | no | 50 | `3a` ×2 · `4a` ×2 · `6a Activity passes` · `12a Deletions/Conflict light` |
| 18 / 14 | no | 49 | `7a Activity quiet` · `7a File lookup` |

§1.3 conflict 7 names two of the four (`2a` 22/20, `7a` 18/14). The **majority** variant, 18/15 across
six frames, is in neither the conflict note nor `02-shell.md`. The mono line appears on exactly the
two frames with the widest padding, which is what the extra 4px is for.

Everything else is invariant across all thirteen: `gap:34px`, centred, `border-top:1px #16181D`, doors
at 13px/**400** (§11's finding again — the prose never names a weight and 400 is what is drawn),
`#828B98` inactive and `#F2F4F7` active. No light frame draws an *active* door, so the light active
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

| Variant | Border | Text | Dot |
| --- | --- | --- | --- |
| idle | none | `--text-label` | `--dot-inert`, 6px |
| syncing | 1px `--border-chrome` | `--text-3` | `--up-to` + `blip 1.6s` |
| n waiting (decisions) | 1px `--chip-attention-border` | `--decision-text` | **1px** ring `--decision` |
| n waiting (deletions) | 1px `--chip-attention-border` | `--decision-text` | filled `--destructive` |
| rehearsal | 1px `--border-chrome` | `--text-3` | none |
| **step N of 2** | none | `--text-5` | none — **and no ⋯ button** |

- **The ring is 1px, not the 2px §2 states** — in both themes.
- **Onboarding drops the ⋯ button too.** §2 says only that the chip is "omitted on onboarding
  (replaced by `step 1 of 2`)". Both `9a` frames have **four** header slots, not five. A menu whose
  only item is the theme toggle has no place in a two-step flow that cannot be left.
- **The border needed a new token.** Measured `rgba(255,107,107,.35)` dark → `rgba(190,18,60,.30)`
  light, against `--decision-border`'s `.32`/`.28`. Close enough to look like the same value and not
  it; the alphas differ per theme, so no `rgba(var(--decision-rgb), …)` form covers both. Added as
  `--chip-attention-border`.
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
closes the window *keeps syncing*, `Ctrl Q` quits *stops syncing*, and the tray must carry those as
sub-labels because *"this is the single worst misunderstanding a tray app can cause"*.

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
*variant* changes, so a count ticking 2 → 3 does not restart the `blip` on its dot. Asserted: focus
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

---

## Phase-1 capability deviations

`IMPLEMENTATION-PLAN.md` §4 lists the ten daemon capabilities the design assumes and which four are
Phase 2. Each Phase-1 fallback gets a row here as its screen lands — G1–G5 close them.
