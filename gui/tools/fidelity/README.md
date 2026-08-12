# The fidelity harness (F8, F9)

What makes "100% fidelity" checkable rather than a claim. Seven gates over the 51 in-scope frames of
`docs/design-v2/Drive Sync.dc.html`.

```
npm run fidelity:extract    # regenerate frames/*.json from the prototype
npm run fidelity            # style, unstamped, fit and hue, the copy gate, then contrast (Chromium)
npm run fidelity:fixtures   # the fixture registry gate                           (Node only)
npm run fidelity:contrast   # the contrast gate on its own; `--report` writes the distribution
```

## The seven gates

| Gate                              | Compares                                                                        | Runs today?                        |
| --------------------------------- | ------------------------------------------------------------------------------- | ---------------------------------- |
| **style** `assert.mjs`            | every mapped app node's computed styles against the drawn node                  | on whatever carries a `data-fid`   |
| **unstamped** `assert.mjs`        | a frame's declared fid slots against the ones the app stamped                   | yes, every declared slot           |
| **fit** `assert.mjs`              | every full window renders at exactly 1040×764, nothing painting over the footer | yes                                |
| **hue** `assert.mjs`              | a settled surface contains no saturated colour anywhere                         | yes, all 5 settled frames          |
| **copy** `copy-gate.mjs`          | every fixed string in `ui/copy.js` appears verbatim in the frames               | yes, every string and 71 templates |
| **fixtures** `check-fixtures.mjs` | every in-scope frame has a dataset, of the shape its class implies              | yes, all 51                        |

The first five need a browser and run in the `fidelity` CI job. The last does not, and runs in
`frontend` alongside the linters — a gate that can run in the fifteen-second job should.

## The hue gate, and the threshold that had to be measured twice (S1)

`03-main-screen.md`: _"No seam. No colour anywhere. This is the rule made visible — if a screen has
nothing to report it must contain no hue at all."_ It is a line item on the design's own acceptance
checklist, assigned to this harness, and nothing implemented it until there was a settled screen to
point it at.

It cannot be tested as "is it grey". **This design's neutral ramp is deliberately cool and not one
tier of it is achromatic** — `#828B98` and `#6D7783` each spread 22 channel values — so a plain
`max − min` test flags the entire palette. It has to be a saturation.

And it cannot be the obvious saturation. **HSL divides the chroma by lightness, so a near-white
neutral reads as vividly coloured**: light's own `--surface` is `#FAF8F5`, a five-value warm tint,
and HSL calls it 0.33 saturated — more than any threshold that still catches `#22D3EE`. The first
version failed `12a Compact settled light` on the surface the whole light theme is painted on. HSV
divides by `max` and is stable at both ends: `#FAF8F5` 0.02, `#C9D0DA` 0.08, and the darkest
neutrals top out at 0.22, against 0.85–0.94 for every accent in either palette.

It reads computed colour PROPERTIES, so it cannot see an image: the app mark is a warm SVG in an
`<img>`, and the frames draw the same one.

## Recorded deviations (S1)

`known-deviations.mjs` holds the assertions a screen cannot pass yet, each with the issue that closes
it, because Phase 1's answer to a missing capability is to **omit the clause, never fake it** — and
an omitted clause arrives here as a node 195px wide where the frame has one 390px wide. Filling it
with plausible numbers is what DEVIATIONS §60 forbids; leaving the gate red makes it a gate nobody
reads. So the exact `frame · node · property` is recorded, never a wildcard, and printed in full on
every run rather than folded into the pass count.

**An entry that no longer fails is itself a failure.** That is the clause that stops the list turning
into somewhere failures go to be forgotten: the day the capability lands, the build fails until the
row is deleted.

## The blocks that render nothing (S7)

The style gate compares STAMPED nodes, so a block the app does not render stamps nothing and is
simply not compared — a screen can render almost nothing and stay green. That is not hypothetical:
S5's `7a Never synced` body rendered empty through four separate causes with every gate passing, and
it was found by dumping `data-fid` attributes by hand.

So `assert.mjs` also compares the other direction: for each frame, the slots its fixture declares
against the ones the app actually stamped. A slot that went unstamped **and whose key the frame
draws** is a block rendering nothing, and it is a failure unless `KNOWN_UNSTAMPED` in
`known-deviations.mjs` names it with the issue that closes it.

**Every frame, not every _mapped_ frame** — the check runs before the unmapped-frame bail-out, and
that ordering is the check. Below it the gate's own failure was inverted: blank half a screen and
the surviving stamps keep the frame in the mapped set, so the missing half is a finding; blank all
of it and the frame drops to the "screen not built" printout and the run goes green. Making
`7a Never synced` stamp nothing gave `35/51 frames mapped, 66362 assertions, 0 failures`, exit 0 —
806 assertions gone, on the frame this mechanism was built for.

That printout is split for the same reason. A frame with **no `fids` map** is a screen nobody has
written yet, and stays an informational line. A frame that **has** a mapping and stamped none of it
is a built screen rendering nothing, and is its own failure — separately from whatever its slots
report, because a mapping whose every key sits past the probe's reach would otherwise stamp nothing,
report nothing, and read as "not built".

**The "and whose key the frame draws" clause is what makes it a gate rather than a printout.** It
began as a report, and eight of the twelve slots it named were noise: `compactFids` is a factory
over four tree shapes and hands every frame the whole slot vocabulary, so `10a Settled` declaring
`meta` says the shape has a meta line, not that this panel draws one. `check-fixtures.mjs` tolerates
exactly that — its rule is "alive somewhere", not "alive here" — and reporting them here both
contradicted it and gave the list a permanent floor of benign lines for a real entry to hide in.
Filtering on the frame's own nodes removes all eight, statically and permanently, and what remains
is four slots that mean what the report says they mean.

These cannot live in `KNOWN_DEVIATIONS`: an unstamped node produces no assertion, so a row there
would never fire, and the rule one section up would reject it for never failing. The staleness rule
is the same though, transposed — a `KNOWN_UNSTAMPED` row that is no longer observed fails the build,
whether because the capability landed, the prototype moved the node, or the frame stopped being
mapped. A row is one NODE — `frame`, `slot` **and** `key` — so a factory slot recorded at one key
never vouches for the rest of its run, and a node that moved fails twice (the row goes stale, the new
key arrives unexplained) instead of being quietly absorbed.

Its 25 rows name seven issues. Six are #98: `2a Syncing`, `2a Needs you` and `12a Syncing light` draw
a 2px progress track under the in-flight transfer, and `TransferActivity` carries `bytes_total` on an
upload and `bytes_done` on a download and never both, so no percentage exists to draw (DEVIATIONS
§63). The rest are per-file sizes the rehearsal does not report (#191 ×5), the deletion facts the
index cannot answer (#208 ×4, #225 ×4), the onboarding account line and remote picker (#241, #99),
and the count of files that already match (#242 ×4).

**Six of the 25 are a light twin of a dark row**, because a row is one NODE and `frame` is part of
its identity: a capability the daemon does not have is missing from `12a Deletions light` exactly as
it is from `4a Deletions`, and nothing here lets the dark row vouch for the light one. That rule is
what found them — S10's first pass read the fid map off the raw registry entry rather than the
resolved one, so all seven light frames iterated an empty mapping and reported six fewer omitted
blocks than the app has. Green, and wrong, on the frames it had just been pointed at (§91).

**Factory slots are probed, not skipped** — the half #247 shipped without (#248). A factory
(`row: (i) => …`) resolves to a different key per call, so it cannot be read off the map; `probeSlot`
calls it over a 10³ index grid and keeps the keys the frame draws. That is stricter than
`check-fixtures.mjs`'s single-axis probe on purpose: `sideRowNote(s, i)` is keyed by side **and** row,
and one axis reaches only row 0 of each side. Measured — the grid finds 39 drawn-but-unstamped slots
where one axis finds 33, and all six extra are further rows of clusters the one axis already found.

Leaving factories out was not free. 218 of the 838 slots declared when this was measured (S8) are
factories, and a factory slot is by
definition a repeated block — a row, a card, a fact, a path — which is exactly what a screen renders
none of. It was also why #247 did not catch the case it was built for: the compact panel declares
`transferTrack`/`transferFill` as factories, so S8 wiring the tray panel to `SyncActivity` passed it
green. It does not pass this — `progress: null` on both `10a Syncing` transfers now exits 1 naming
all four nodes.

Two things the probe still cannot reach, and both fail **safe** — the key is never produced, so the
slot is never reported: an index past `PROBE_DEPTH`, and a factory wanting a non-numeric argument.
Neither bites today, and both were checked rather than assumed: raising the depth to 30 reaches not
one drawn key that 10 does not, and every fid factory takes at most three arguments and is keyed by
position.

## The node key, and why it is a path

`data-fid="<frame-label>:<node-key>"`, where the key is a slash path of tag plus an index among
same-tag siblings — `header/img`, `div[2]/div[0]/span[1]`.

The F8 issue asks that this decision be written down, and offers a hand-maintained mapping file as
the alternative. One argument settles it: **the app's tree is not the prototype's tree and never
will be.** F4 already wraps the app mark in a `<button>` the frames draw bare, and every screen will
diverge further. So a key can only ever be an identifier a human attaches to the app node they judge
to correspond — there is no automatic correspondence to be had. Given that, the identifier should at
least be derivable, diffable and unambiguous, which a list of hand-chosen names is not.

The accepted cost, stated in the issue: a prototype edit that changes structure renames keys, and
the `frames/*.json` diff shows it. That is a review, not a silent break — `assert.mjs` fails loudly
on a `data-fid` naming a key that no longer exists.

The other half of the mapping — _which_ app node is `header/span[2]` — lives in
`src/js/fixtures/frames.js` as each frame's `fids`, and `ui/chrome.js` stamps it.

## Why it renders instead of parsing

The issue says `extract.mjs` "parses" the prototype. It renders it, in the same engine, and that is
the reason the output is usable: `assert.mjs` reads the app off `getComputedStyle`, so the
prototype's side has to come from the same place. A parse gives you `padding:0 20px` where the app
gives you four longhand pixel values, with every cascade, inheritance and default missing.

## Engine

**Chromium, via puppeteer.** The app's real runtime is WebKitGTK and the issue asks for Playwright's
WebKit, which is the right instinct — a gate should run in the engine that ships. It is not usable
here: Playwright's WebKit needs `libicu74` and `libjpeg-turbo8` installed through `sudo`, and a gate
nobody can run locally is a gate nobody develops against. Both scripts take their launcher in one
place; switching is a small change, and CI is the right place to add WebKit alongside Chromium when
the runner allows it.

Every measurement in F1–F4 came from this engine, so the numbers in `DEVIATIONS.md` and the numbers
here are consistent with each other.

## Tolerances

- **Colours** exact. Both sides are already `rgb()`/`rgba()` — no parsing, no rounding.
- **Lengths** ±0.5px.
- **`font-family`** first name only. The fallback stack is a deployment detail.
- **`line-height: normal`** is a wildcard. Resolving it compares font metrics rather than design,
  and F1 established that font metrics move every measurement in this design.
- **Size** is compared as a border box (`getBoundingClientRect`), never as the computed `width`.
  The prototype does not opt into `border-box` and `base.css` opts the app in globally, so the same
  element reports `1000px` in one document and `1040px` in the other while occupying an identical
  1040px on screen. See `BORDER_BOX_INSET`.
- Properties at their CSS initial value are **omitted** from the fixtures and read back as that
  initial (`INITIAL` in `props.mjs`). Lossless, and the difference between a 4.4 MB dump and a diff
  a human will read. Inherited properties are never omitted — "absent" could not be resolved.

## Frame classes

They are not all windows.

| Class          | Count | Asserted                                                                      |
| -------------- | ----- | ----------------------------------------------------------------------------- |
| `window`       | 20    | everything, including the fit gate                                            |
| `dialog`       | 10    | everything at its own size                                                    |
| `compact`      | 11    | everything; note the panel is drawn **362** wide, not 360                     |
| `notification` | 4     | everything except fit — the desktop sizes a banner                            |
| `crop`         | 2     | everything except its own width; drawn at 600 inside the 1040 Settings window |
| `specimen`     | 4     | only the inner artefact; the wallpaper and taskbar are scenery                |

## Pinning the environment

Three of this harness's first four CI failures were the environment leaking into the measurement,
never the code. A fidelity gate that does not pin its environment measures its environment, so both
scripts fix the same things at the same call sites:

| Pinned                                      | Why                                                                                                                                                                                                                      |
| ------------------------------------------- | ------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------------ |
| viewport `1040×764`, `deviceScaleFactor: 1` | a length must be one CSS pixel, rounded once                                                                                                                                                                             |
| `prefers-color-scheme` **per frame**        | the `12a` set is the light theme; everything else is dark. A headless browser's default is a property of the platform, and unpinned this compared dark frames to a light app — 187 failures no developer could reproduce |
| `prefers-reduced-motion: no-preference`     | four app stylesheets answer it and every one sets `animation: none`, and animation properties are asserted. The prototype has no reduced-motion rules at all                                                             |
| `@font-face` injected into the prototype    | it declares the families and has no `@font-face`, so unpinned it renders in whatever the machine falls back to                                                                                                           |
| every animation seeked to 0 and paused      | otherwise the harness records animation _phase_: `opacity` 0.82 one run and 0.79 the next, and a `blip` dot measured 8.8px because the reading caught the 1.5× transform mid-cycle                                       |

`forced-colors` is **not** pinned — puppeteer rejects it as unsupported. Nothing in the app answers
it today, so nothing is unpinned in practice, but the first stylesheet that does will need a way.

Animations are frozen through the Web Animations API rather than an `animation-play-state: paused`
override, because the declarations are themselves asserted. Overriding a property in order to
measure it is how a gate ends up agreeing with itself.

### The one thing that cannot be pinned

**Glyphs no bundled font provides.** The design uses `⇄`, `⌕`, `⊘`, `▲◐▮▾` and — least obviously —
`＋`, the FULLWIDTH plus sign, which looks like `+` and is nowhere near it in Unicode. F1 vendored
the latin and latin-ext subsets; those are outside them. Their advance widths come from whatever the
machine has installed and differ by whole pixels: `10.89` here, `8.47` on ubuntu-latest.

Coverage is read from `base.css`'s own `unicode-range` declarations rather than a hand-written list
of blocks. The hand-written version is what missed `＋`, and it would have gone stale the moment F1's
subsets changed. 246 nodes across the 51 frames are flagged.

An unbundled glyph does not only corrupt its own width — **it moves its neighbours**. `10a Syncing`
draws a filename and a `→` in one flex row; the arrow measured 12px here and 10.06px on
ubuntu-latest, and the 1.94px it gave up landed on the filename beside it, which contains nothing but
Latin. Exempting only the node holding the glyph left its neighbour failing.

So the rule follows the layout: **a box is comparable only if no unbundled glyph appears anywhere
inside its parent's subtree** (`boxComparability`). That covers the node itself, every sibling it
shares flex or grid space with, and every ancestor whose size sums it.

Everything else about those nodes is still asserted — colour, padding, font-size, position, border.
Only the size they happen to occupy is not. Nothing can make this deterministic except bundling a
symbol font, which would change what ships.

## What this cannot cover, ever

Stated rather than implied, because a gate that seems to cover more than it does is worse than one
that admits its edges:

- **The seven screens with no drawn light frame.** Their light theme is `12-light-theme.md`'s
  mapping table applied at the token layer, which is prose, not a drawn artefact. `check-tokens.mjs`
  proves the mechanical half and `check-contrast.mjs` proves the part that matters — that no token
  landed on the wrong end of its ramp — but neither is a drawn frame and neither pretends to be.
- **A stroke's light value on an undrawn screen.** The contrast gate reads text; a hexagon track is
  drawn a shade off the surface on purpose and a legibility gate has no opinion about one. Strokes
  ARE compared exactly on the eight light frames that exist.
- **Whether an animation looks right.** Only the declaration is comparable — name, duration, delay,
  timing function. A wrong easing that parses is invisible here.
- **Native tray rendering.** Not a webview; it has no DOM. The tray strings are exempted in
  `copy-gate.mjs` with that reason.
- **The desktop's own notification chrome.** Only the banner's content is ours.
- **Motion, focus order and hover states.** The gate reads a static tree.

## State of play

`assert.mjs` reports how many frames carry a `data-fid` and lists the rest every run, so "the gate is
green" can never be confused with "the gate looked at anything". **S10 took it to 51 of 51 and
94,299 assertions**, and the last eight arrived together because they are one task: the `12a` set is
the light theme, and a light frame could not be mapped at all until the ground truth stopped
recording the prototype's dark page as the frame's own colour (§58b, §91).

That last frame count is the one number in this file that should be read with its companion. **628
colour comparisons are declined on those eight frames** — printed per frame, every run — because the
prototype never set them. A light frame is compared on everything it declares and on nothing it
inherits, which is less than a dark frame is compared on, and the gate says so rather than letting
51/51 imply otherwise.

**S1 moved the assertion count by 5,296 and the frame count by zero**, which is the honest shape of
what it did: `2a Settled`, `2a Syncing` and `2a Needs you` were already "mapped" on the strength of a
header and a footer, and now have a hero, a seam, two side labels, a transfer row and an attention
band mapped as well. A frame counter cannot tell the difference between a screen and a title bar.

**S4 moved both**: three frames and 9,778 assertions (33,098 → 42,876), which is the shape of a screen
whose largest state is a nine-row list. It also moved the deviation count from 24 to 48 — every one of
those a Phase-1 capability the daemon does not have, recorded with the issue that closes it rather
than left to fail. Two thirds of them are one fact: `5a Checking` is a 522px window and the shell's is
a fixed 1040.

**All 51 have a dataset** (F9), which is a different claim and deliberately kept separate: a fixture
is what the app is fed, a `data-fid` is what gets compared. `check-fixtures.mjs` proves the first,
`assert.mjs` counts the second, and neither number can inflate the other. Adding the 40 datasets
moved 11/51 not at all. The two counts only met at S10, and by inheritance rather than by writing:
a light frame's mapping IS its dark twin's, because the two are one tree drawn twice — checked, not
assumed, by `check-fixtures.mjs`'s fifth check.

F6 was the first task to put a hexagon, a transfer row and an SVG colour in front of the gate, and it
found three things wrong with them rather than with itself; `DEVIATIONS.md` §58c has all three. The
one that changed this harness: **`fill` and `stroke` are compared as the engine computes them, not as
attribute strings.** The prototype writes `#2E323A` and the app writes `var(--hex-settled-track)`,
because `tokens.css` is the only file allowed a raw colour and light is a token swap — compared
literally, no themed mark could ever pass. `var()` resolves inside a presentation attribute, so both
sides come out `rgb(46, 50, 58)`, which is the same footing every style property is already on. A
`url(#id)` reference matches any other: the id must be unique per instance (`10a Glyph states` draws
ten marks on one page), so it is not design.

**What the reference points at is compared, and that is what pays for the wildcard** (#204).
`stop-color` is a style property on both sides — the prototype writes the presentation attribute
`stop-color="#E55B2B"`, the app writes `style="stop-color:var(--up-from)"`, and both compute to
`rgb(229, 91, 43)`. `offset` and `x1`/`y1`/`x2`/`y2` are attributes on both. The last four are what
make an up gradient an up gradient (`0,0 → 1,1` against `0,1 → 1,0`), so a syncing mark with its two
directions swapped now fails with sixteen assertions rather than passing silently — verified by
swapping them.

Building the harness before the screens is deliberate: each S-task's definition of done is "my
frames pass", and eleven screens written against no gate at all would be eleven screens to re-check
afterwards. It found six classes of drift in the shell on its first live run.

## The fixtures, and the preview that shares them (F9)

`src/js/fixtures/` holds one deterministic dataset per in-scope frame label, selected by
`?frame=<label>`. The same data drives this harness and the browser design preview, and the pairing
is the point: **a frame that passes CI is a frame a human can open and look at**, so "green" stays
falsifiable by eye. A gate whose inputs nobody can see is a gate nobody trusts.

```
?frames                 the index — every in-scope frame, linked
?frame=<label>          that frame's dataset, served to every command by api.js's mock
?theme=light|dark       an explicit override, for a light frame on a dark machine
```

One module per screen family (`conflicts.js`, `deletions.js`, …), plus `fids.js` for the node-key
tables and `clock.js` for the one value a fixture may read from the wall clock. `frames.js` is the
registry that assembles them and the only module `ui/` imports.

**What a fixture may not do.** Compute a displayed string — anything drawn is either a literal or is
rendered by the app from a number the fixture pins. Relative renders (`2 minutes ago`) are pinned as
`ago(120)`; absolute ones (`14:38`) are written literally, because an epoch formatted as a clock time
depends on the machine's timezone and moves across midnight. `check-fixtures.mjs` fails on a `new
Date()` anywhere in the directory.

**What a fixture may not invent.** Four frames draw content no Phase-1 command returns — the engine
gaps G1–G4. A plausible-looking field shape for those would pre-empt a design nobody has agreed to,
so the fixture carries only the Phase-1 surface and `DEVIATIONS.md` carries the difference.

`fids` is not part of a dataset. It maps app nodes onto drawn nodes, which needs the screen to exist,
so each S-task adds its own — and `check-fixtures.mjs` fails on a slot whose key exists in no frame
that declares it, which is how F6's dead `hexRect`/`hexNumeral` declarations were found.

## Determinism, checked rather than assumed

```
npm run fidelity:determinism   # extract twice, require byte-identical output
```

Every cross-machine failure this harness has had was an unpinned input, and the cheapest way to find
one is to look for it on a single machine first. Two extractions of the same prototype seconds apart
must produce identical bytes; when they do not, something is being measured that is not the design.

It found the last one: `opacity` under `breathe` read `0.45` on one run and `0.450015` on the next.
The animation freeze was seeking and _then_ pausing, which leaves a gap for the compositor to
advance — and the freeze ran in a separate evaluation from the measurement, which leaves a second
one. Pausing first and freezing inside the measuring call closed both.
