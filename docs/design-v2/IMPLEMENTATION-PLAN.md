# Implementation plan — design-v2

**Tracking:** #195 · milestones *Desktop UI v2* (#162–#189) and *Desktop UI v2 — engine* (#190–#194).

How the `docs/design-v2` bundle gets built, and how "100% fidelity on all screens" becomes a
thing a machine can check rather than a claim.

**Scope decisions taken before writing this plan:**

| Decision | Choice |
| --- | --- |
| Daemon capability gaps | **Phased.** Phase 1 builds every frame; capabilities that are cheap GUI-side get built now, genuinely engine-level ones ship in Phase 2 with the documented fallback in between. |
| Fidelity gate | **Computed-style assertions per frame**, in CI. |
| `#6D7783` quiet tier (4.33:1) | **Keep as drawn.** Logged as a known accessibility deviation. |
| Delivery | **Milestone "Desktop UI v2"**, foundation issues then one issue + PR per screen. |

---

## 1. The fidelity contract

### 1.1 What "all screens" means, exactly

`Drive Sync.dc.html` contains **60** frames carrying a `data-screen-label`. **51 are in scope.**

| Group | Frames | Size |
| --- | --- | --- |
| **2a Main screen** | Settled · Syncing · Needs you · Compact settled · Compact syncing · Compact needs you | 3×1040×764, 3×360 |
| **3a Conflicts** | Conflict · Conflict diff · Conflicts cleared | 2×1040×764, 520×764 |
| **4a Deletions** | Deletions · Armed · Empty · Compact | 2×1040×764, 520×420, 360 |
| **5a Plan a sync** | Plan · Plan safe · Checking | 2×1040×764, 520×764 |
| **6a (partial)** | Activity passes · Details | 1040×764, 520×460 |
| **7a Activity** | Activity quiet · File lookup · Never synced · File pending | 2×1040×764, 600×600, 600 |
| **8a Settings** | Settings · Skip rules · Deletions tab · Schedule monthly · Save refused | 2×1040×764, 600×520, 2×600 |
| **9a Onboarding** | Folders · Review · First sync · Consent · CLI missing | 2×1040×764, 600×540, 2×600 |
| **10a Tray** | In situ · Glyph states · Settled · Syncing · Offline · Paused | 1040×520, 560, 4×360 |
| **11a Notifications** | In situ (4 banners) · Rules · Settings · Outage · Grouped | 1040×560, 600, 3×520 |
| **12a Light theme** | Settled · Syncing · Deletions · Conflict · Compact settled · Compact syncing · Compact needs | 4×1040×764, 3×360 |
| **12a Tray light** | Specimen card, not a product surface — see §1.2 | 360 |

**Out of scope — 9 frames, and why:**

- `1a Main`, `1a Compact`, `1b Main`, `1b Compact`, `1c Main`, `1c Compact settled`,
  `1c Compact attention` — round-one exploration, kept in the prototype for reference.
  `README.md` calls 2a the design that supersedes them.
- `6a Activity files`, `6a Quiet` — the tide-chart Activity. `07-activity.md` states outright:
  *"kept in the prototype as `6a` for reference but is not the spec."* The other two 6a frames
  (`Activity passes`, `Details`) **are** normative — `07-activity.md` specifies both in full.

### 1.2 Frame classes — they are not all windows

The harness must treat four different kinds of frame differently:

| Class | Frames | Assert |
| --- | --- | --- |
| **Full window** — 1040×764 | 2a×3, 3a×2, 4a×2, 5a×2, 6a passes, 7a×2, 8a×2, 9a×2, 12a×4 | Everything, including the frame's own box. Must fit with no clipping. |
| **Standalone dialog** — own chrome + shadow | 3a cleared, 4a Empty, 5a Checking, 6a Details, 7a Never synced/File pending, 8a Save refused, 9a First sync/Consent/CLI missing, 11a Rules/Settings/Outage/Grouped | Everything, including width/height. |
| **Compact panel** — 360px | 2a×3, 4a Compact, 10a×4, 12a×3 | Everything. Shared component; the tray reuses it. |
| **Content crop** — drawn at 600px but lives inside a 1040 screen | `8a Deletions tab`, `8a Schedule monthly` | Everything **except** the frame's own width — the parent screen sets that. |
| **In-situ / specimen** — desktop mock or swatch card around a real artefact | `10a In situ`, `11a In situ`, `10a Glyph states`, `12a Tray light` | Only the inner artefact (tray panel, the four 372px banners, the glyph SVGs). The GNOME top bar and the specimen card's own chrome are drawing furniture, not product. |

**`12a Tray light` is a specimen, not a light compact panel.** Its outer card is *dark*
(`#0E0F12`, `radius 12`, `padding 18px 20px`) and holds a light GNOME top-bar strip plus a
paragraph of prose. The only product content is the 14px needs-you glyph inverted for a light
panel — `stroke:#14161A`, `stroke-width:9`, `circle cx=60 cy=60 r=17` filled. Assert that and
nothing else. **Light-theme product frames therefore number 7, not 8** — which matters, because
S10's mechanical mapping is what covers the rest.

### 1.3 Precedence, when the spec disagrees with itself

The docs and the prototype **already conflict in places**. Fixed rule, applied everywhere:

1. **The `.md` files are normative** for tokens, rules, semantics and copy.
2. **The matching 1040/600/520/360 frame is normative** for that screen's layout geometry —
   padding, gaps, offsets, per-element colour.
3. **The illustrative swatches** in the prototype's "The system" header block are **not
   normative**. They render at 66px and were drawn before the frames settled.
4. Every conflict is recorded in `docs/design-v2/DEVIATIONS.md` with the resolution, so no one
   re-litigates it mid-build.

**Conflicts already found (P0.2 sweeps for the rest):**

| # | Detail | Doc says | Frame says | Resolution |
| --- | --- | --- | --- | --- |
| 1 | Paused hexagon track | `#2A2E36` (`01-foundations.md` §6) | `#2E323A` (`10a Paused`) | Frame — rule 2 |
| 2 | Settled check path | `M49 60 L57 68 L72 52` @3.6 | swatch: `M50 60 L57 67 L71 53` @4 | Doc **and** the 1040 frames agree; swatch is rule 3 |
| 3 | Paused hexagon bars | `rect w=5.5 h=22 rx=2.5`, fill `#99A2AE` | swatch: `w=5 h=22 rx=2`, `#828B98`, no dasharray, `opacity:.4` | Doc + `10a Paused` agree; swatch is rule 3 |
| 4 | Settled `Sync now` button | secondary, `#101216` fill (`03-main-screen.md`) | `background:transparent`, text `#C9D0DA` (`2a Settled`) | Frame — rule 2 |
| 5 | Syncing hexagon inert track | `fill` unspecified | `fill:#0A0B0D` — it masks the seam behind the mark | Frame; **required**, or the seam shows through the hexagon |
| 6 | Seam bottom offset (main) | `-114..-150px` | `-150px` (`2a Syncing`) | Frame |
| 7 | Footer nav padding | `0 40px 18–22px`, `padding-top:14–20px` | `2a`: `22px`/`20px` + mono line · `7a`: `18px`/`14px`, no mono line | Per-frame; the mono line is optional per `02-shell.md` |

### 1.4 The gate

`gui/tools/fidelity/` — two scripts and a CI job.

**`extract.mjs`** parses `Drive Sync.dc.html`, walks each in-scope `[data-screen-label]` subtree and
emits `frames/<label>.json`: a normalized node tree of `{key, tag, text, styles, svgAttrs}` with
inline styles expanded to longhand. Checked in, regenerated on demand, diffable — so a change to
the prototype shows up as a reviewable diff.

**`assert.mjs`** serves `gui/src` statically, opens each frame's fixture URL in headless WebKit
(Playwright — same engine family as WebKitGTK), and asserts `getComputedStyle` on each mapped node.

**Mapping is explicit on the app side.** Every element the harness checks carries
`data-fid="<frame-label>:<node-key>"`. Structural matching against a hand-drawn prototype is
fragile; an explicit key is greppable, reviewable, and makes an unmapped node a visible gap rather
than a silent pass.

**Where `node-key` comes from — decide this before writing `extract.mjs`.** The prototype carries
no such attributes, so the key has to be derived. Plan: derive a **path-based key**
(`hero/hexagon`, `footer/nav/door[2]`) and accept that regenerating `frames/*.json` after a
prototype edit produces a reviewable diff which may rename keys and require matching `data-fid`
updates. That trade is deliberate — the alternative is a hand-maintained mapping file, which is
more stable but must be edited twice for every change. What must not happen is leaving it
implicit: pick one in F8 and write it into the harness README.

**Inherited typographic defaults will otherwise generate systematic false failures.** Prototype
nodes mostly omit `line-height`, so it computes to `normal`; if `base.css` sets a global
`line-height` — or any inherited type default the prototype lacks — every unstyled node diverges
and the gate is noisy from day one, which is how gates get switched off. Rule for F1/F8:
`base.css` sets **no inherited typographic default the prototype doesn't also set**, and the
harness treats a prototype-side `normal` as a wildcard. Same care for `font-family`, which the
prototype inherits from its outer page wrapper rather than declaring per node.

**Asserted properties:** `font-family` (first family only) · `font-size` · `font-weight` ·
`letter-spacing` · `line-height` · `text-transform` · `color` · `background-image` and
`background-color` · `border-*` (width/style/colour/radius, per side) · `padding-*` · `margin-*` ·
`gap` · `width`/`height` · `display` · `flex-direction` · `align-items` · `justify-content` ·
`grid-template-columns` · `position` + `top/right/bottom/left` · `opacity` · `overflow` ·
`animation-name`/`duration`/`delay`. For SVG: `d`, `stroke`, `stroke-width`, `stroke-dasharray`,
`stroke-linecap`, `stroke-linejoin`, `fill`, `viewBox`, plus `x`/`y`/`text-anchor` on numerals.

**Tolerances:** colours exact after normalization to `rgb()`; lengths ±0.5px (sub-pixel rounding);
`font-family` matches on the first name. Anything else exact.

**Two more gates in the same job:**

- **Copy gate.** Every string in `13-copy-deck.md` must appear verbatim in the DOM of the frame
  that owns it. Catches a smart quote, a stray "pending" where the deck says "waiting", an em-dash
  turned into a hyphen. The copy is load-bearing around deletion — this is not cosmetic.
- **Fit gate.** Every 1040×764 frame renders at exactly 1040×764 with
  `scrollWidth/scrollHeight === clientWidth/clientHeight` on the window root, and no descendant
  overflowing the footer. `02-shell.md` records this bug being found twice during the design.

**What this gate cannot cover — stated, not hidden:**

- Seven of the eleven screens have **no light-theme frame drawn**. Their light theme is asserted
  against the `12-light-theme.md` mapping table applied mechanically to the dark frame — a
  token-substitution check, not a comparison to a drawn artefact.
- Animation *motion* (only the declaration is checked), native tray rendering, and the desktop's
  own notification chrome are verified by review, not by the harness.
- Content crops can't have their width asserted (§1.2).

---

## 2. Prerequisites — blocking, do these first

**Fonts.** The design is Instrument Sans + IBM Plex Mono. The current `tokens.css` names IBM Plex
*Sans* and the woff2 files were never bundled, so the app renders in system fonts today. Font
metrics move every single measurement in the spec — **no pixel comparison before this is real.**
Add `@fontsource/instrument-sans` and `@fontsource/ibm-plex-mono` as devDependencies, copy the
Latin woff2 subsets into `gui/src/fonts/`, and `@font-face` them in `tokens.css`. Both are OFL.
The CSP already forbids external font hosts, and this is a desktop app that must render offline.

**Window geometry.** `tauri.conf.json` is `resizable: true, minWidth: 900, minHeight: 600`. Every
frame is drawn at 1040×764 and `01-foundations.md` §4 says the design assumes the fixed window for
now. Set `resizable: false`, `width: 1040`, `height: 764`. The reflow rules for a resizable window
(seam stays at 50%, columns stack below ~880px, hairline dropped) are written down in §4 — file
them as a follow-up issue rather than building against an unfinished assumption.

**Commit the bundle.** The `ui-design` worktree currently has `docs/design/` deleted-but-uncommitted
and `docs/design-v2/` + `docs/design-v1-old/` untracked. Land that as commit one so every
subsequent PR has a stable reference to point at.

---

## 3. Architecture

### 3.1 What survives

Per `README.md` and `14-behaviour-and-state.md`: **the daemon interface is unchanged.** This is a
presentation layer over the same socket. Keep `gui/src/js/api.js`'s command surface, `gui-core`'s
typed boundary, the vanilla-JS + hand-rolled-DOM approach, the screen-module pattern, and
`tokens.css` as the single source of visual truth. **No framework** — the prototypes are only
React-flavoured because of the tool they were drawn in.

Also explicitly kept (`01-foundations.md` §9): the `journalctl` escape hatch and its wording, the
`.sync` always-ignored rule, surgical config writes, the typed-`DELETE` gate, and the
`.proton-cloud` conflict suffix.

### 3.2 What's replaced

The 214px sidebar, the seven-item nav registry, the permanent orange safety banner, the four stat
tiles, the seven-filter activity ledger, the flat-top hexagon with nested arcs, the `◐ Theme`
titlebar button, `state-matrix.js`, `screens/overview.js` and `screens/history.js` (folded into
Activity). `components.js` splits into a `ui/` library.

### 3.3 Routing — four doors, more surfaces

`02-shell.md`: the four footer doors are **Activity · Plan a sync · Settings · Details**, and they
never move or reorder. Conflicts and Deletions are **not** nav destinations any more — they are
reached from the attention band, the status chip, or a notification. `Details` opens a 520×460
dialog, not a tab.

So the router carries three kinds of route:

- **Root** — the main screen. Default; no door of its own.
- **Door routes** — Activity, Plan a sync, Settings.
- **Overlay routes** — Details, Conflicts, Deletions, Never-synced, Save refused, Armed
  confirmation, and the onboarding takeover.

> **Open question for the designer.** No frame shows how you return to the main screen from
> Activity or Settings. Plan assumes: **clicking the active door returns to root**, and the app
> mark in the header is also a home affordance. `3a Conflicts cleared` has an explicit
> `Back to sync`. Flagged, cheap to change.

### 3.4 Module layout

```
gui/src/fonts/                instrument-sans-{500,600,700}.woff2, ibm-plex-mono-{400,600}.woff2
gui/src/styles/
  tokens.css                  full design-v2 palette, dark + light, both blocks
  base.css                    reset, @font-face, focus ring, prefers-reduced-motion
  shell.css                   header, status chip, footer nav, footer action bar, content
  components.css              hexagon, seam, buttons, inputs, cards, bands, rows, dialogs
  screens/*.css               one per screen
gui/src/js/
  app.js                      shell + router + poll loop + keyboard map
  routes.js                   route table (root / door / overlay)
  api.js                      unchanged command surface; fixture-aware mock
  store.js                    extended selectors
  fixtures/                   one deterministic fixture per frame label
  ui/
    el.js                     the DOM builder (moved out of components.js)
    hexagon.js                5 states + warning variant, size→stroke table, numerals
    seam.js                   gradient, per-height stops, mask helper, stop-above-band
    chrome.js                 header, status chip, ⋯ menu, footer nav, footer action bar
    controls.js               buttons, inputs, toggle, segmented, pills, day chips, stepper,
                              radio cards, checkbox, typed-DELETE gate
    rows.js                   transfer / action / fact / pass / history rows, deletion cards
    bands.js                  attention, destructive, never-synced, consent
    dialog.js                 overlay layer: sizing, Esc, focus trap, backdrop
    compact.js                the 360px panel — five states, shared with the tray window
    format.js                 bytes, thousands separators, relative time, plain-English outcomes
    copy.js                   every string from 13-copy-deck.md, as constants
  screens/                    main · conflicts · deletions · plan · activity · settings ·
                              details · onboarding
gui/tools/fidelity/           extract.mjs · assert.mjs · frames/*.json
```

`copy.js` matters more than it looks: the copy deck is the spec, the copy gate asserts against it,
and one module means a string can't drift between the screen and the tray and the notification
that quote the same sentence.

**No bundler.** Tauri serves `gui/src` raw, so every import specifier keeps its `.js`
(`import-x/extensions: always`) — a dropped extension is a blank window at runtime, not a build
error.

### 3.5 The hexagon and the seam are the two load-bearing primitives

Everything else is composition. Build them first, and build them right:

- **Pointy-top**, `viewBox 0 0 120 120`,
  `d="M60 9.4 L103.1 33.8 L103.1 86.3 L60 110.6 L16.9 86.3 L16.9 33.8 Z"`,
  `stroke-linejoin:round`. Perimeter ≈297 — the number the dash arrays are tuned against.
  Getting flat-top instead is, per the spec, the single easiest way to make the redesign look
  off-brand.
- Nothing nested inside it. The outline *is* the animation track.
- Sizes 176/168/132/116/104/96/88/80/76/74/72/52/46/44/34/20/15/14/13, with stroke width scaling
  so the mark reads the same weight at every size (3.4 @168 → 9–12 @13–20).
- Five states + the warning variant. `10-tray.md`: **only five forms exist** — a solid filled
  hexagon is not a state and must not be reintroduced.
- Seam: 1px at `left:50%`, gradient fading to surface at both ends, never touching an edge.
  Four hard rules — drawn only when it means something, stops above any full-width band, anything
  centred on it gets an opaque background mask (`z-index` alone is not enough), and direction is
  carried by position first and colour second.

---

## 4. Daemon capability gaps — what's cheap, what's engine work

`14-behaviour-and-state.md` lists ten capabilities the design assumes. On inspection **six are
cheap GUI-side work** and belong in Phase 1; only four are genuine engine work. That materially
raises what Phase 1 can hit.

| # | Capability | Frames it drives | Verdict |
| --- | --- | --- | --- |
| 1 | `deletion_policy` | `8a Deletions tab` | **Phase 1.** Maps exactly onto the existing `[delete_approval] remote/local` keys: *Ask me every time* = both `true`; *Only ask about permanent ones* = `remote=false, local=true` (the recoverable direction stops asking, the permanent one keeps asking); *Never ask* = both `false`. The tab ships in full. Two things to log rather than build for: the mono key line `deletion_policy · applies to both directions` deviates (fix in G5 with a `config.rs` alias), and `dirconfig.rs` lets a `.proton-sync.toml` in any directory override this daemon-wide default — the tab writes the global value and the UI has no surface for per-directory overrides. |
| 2 | Live skip-rule match counts | `8a Skip rules` (the whole effect column, `hiding 4 files, 3.1 GB`) | **Phase 1.** `gui-core/src/index_read.rs` already reads the index; match each exclude glob against it for counts, bytes, sample paths, and the `Matching nothing` stale marker. This is the point of the tab. |
| 3 | Prose diff summary | `3a Conflict` version cards, items 1 and 2 | **Phase 1.** `read_conflict_pair` already returns both texts and mtimes. A line-level classifier gives "You added a line, 5 minutes ago" and "Yours has `buy milk` where Proton's has something else". Fallback if it can't classify: metadata row only — **never** the raw diff, that's what the disclosure is for. |
| 4 | Free-space check | `9a Review` — `Needs 38.4 GB free. You have 214 GB.` | **Phase 1.** `statvfs` on the local root in a Tauri command. |
| 5 | Distro detection | `9a CLI missing` | **Phase 1.** `/etc/os-release`. Fall back to tarball instructions rather than guessing a package manager. |
| 6 | `notify_policy` | `11a Settings` | **Phase 1.** GUI-local setting. **"Never" must not change engine behaviour** — deletions still wait for approval. Turning off notifications is not consent. |
| 7 | Per-file state + history | `7a File lookup` history block, `7a File pending` | **Phase 2 (G1).** Needs a per-path history query. Phase 1 ships the verdict + both side cards from `path_sync_status`, and the pending variant from `SyncActivity`; the history block is omitted. |
| 8 | Byte totals per direction, per window | `2a Syncing` footer line, `7a` seam counts | **Phase 2 (G2).** Daemon counters. Phase 1 omits the footer totals line (the spec's own fallback). |
| 9 | Filtered apply | `5a Plan` — `Run it without the deletion` | **Phase 2 (G3).** Relates to open issue #100 (plan token). Phase 1 hides the button rather than faking it — the spec is explicit about that. |
| 10 | `full_scan_schedule` | `8a Settings` panel 2 — segmented control, day chips, time stepper | **Phase 2 (G4).** Phase 1 presents the existing `scan_interval` in plain language ("every N minutes") inside the same panel shell. This is the largest single Phase-1 fidelity deviation. |

Every Phase-1 deviation gets a line in `DEVIATIONS.md` naming the frame, what differs, and the
Phase-2 issue that closes it.

---

## 5. Work packages

### Phase 0 — bundle and reconciliation

| Task | Definition of done |
| --- | --- |
| **P0.1** Commit the design bundle | `docs/design-v2/` (including this plan) + `docs/design-v1-old/` tracked; old `docs/design/` deletion committed |
| **P0.2** Reconciliation sweep | All 14 docs swept against their frames; every numeric/colour/copy conflict in `DEVIATIONS.md` with a resolution under the §1.3 precedence rule. **Includes validating `12-light-theme.md`'s mapping table against the four drawn light windows** — S10 propagates that table to seven screens with no frame to catch an error in it, so a wrong row there is expensive later and cheap to find now |
| **P0.3** Decisions record | Quiet tier, fixed window, routing back-to-root, precedence rule written down |

### Phase 1 — foundations

| Task | Delivers | Done when |
| --- | --- | --- |
| **F1** Fonts + tokens | Bundled woff2; `tokens.css` rewritten with every token in `01-foundations.md` §1 and `12-light-theme.md`; `base.css` with reset, focus ring (`2px #3B82F6` at `2px` offset), `prefers-reduced-motion` | Both theme blocks complete; `prefers-color-scheme` default with explicit choice persisted; no raw hex outside `tokens.css` |
| **F2** Hexagon | `ui/hexagon.js` — 5 states + warning variant, full size→stroke table, centred mono numerals, `hexup`/`hexdn`/`breathe`/`blip` keyframes, reduced-motion degradation | Renders every size in the spec; pointy-top; no nested shapes; syncing track carries `fill:<surface>` |
| **F3** Seam | `ui/seam.js` — gradient with per-height stops, mask helper, stop-above-band | All four rules enforced; a mask helper that any centred element can wear |
| **F4** Shell + router | Header (52px, no bottom border), status chip (5 variants), `⋯` menu incl. the theme toggle, four-door footer, footer action bar, content area with the `overflow` discipline, overlay layer, keyboard map | Doors never move; `Ctrl F`/`Esc`/`←→`/`Ctrl S`/`Ctrl ,`/`Ctrl W`/`Ctrl Q` all wired |
| **F5** Controls + rows + bands | `ui/controls.js`, `ui/rows.js`, `ui/bands.js`, `ui/dialog.js` | Every button kind from `01-foundations.md` §1; hover/press/focus/disabled/selected per `14-behaviour-and-state.md` §"Interactive states" |
| **F6** Compact panel | `ui/compact.js` — 360px, five states | Drives `2a` compacts, `4a Compact`, all four `10a` panels |
| **F7** Copy + formatters | `ui/copy.js` (the whole deck), `ui/format.js` | Copy gate passes against `13-copy-deck.md` |
| **F8** Fidelity harness | `gui/tools/fidelity/` + `frames/*.json` + CI job | Style gate, copy gate and fit gate all run; a deliberately-wrong hex fails the build |
| **F9** Fixtures | `js/fixtures/` — one deterministic dataset per frame label, `?frame=<label>` | Every in-scope frame reproducible in browser preview and in the harness |

### Phase 1 — screens

Each screen's DoD is the same shape: **every frame in its row passes all three gates, in both
themes, at 1040×764, with reduced-motion honoured** — plus its own notes.

| Task | Screen | Frames | Notes |
| --- | --- | --- | --- |
| **S1** | Main screen | `2a` ×6, `12a Settled/Syncing/Compact ×3` | The three-state skeleton where the hexagon never moves. Seam/labels/columns fade in 320ms; hexagon crossfades 220ms. Settled screen must contain **no hue at all**. Attention band is additive over syncing, never a replacement. |
| **S2** | Conflicts | `3a` ×3, `12a Conflict light` | **Keep both is the maximum-contrast button** — the safe choice is the loud one. Diff view turns the seam into the diff gutter (`1fr 1px 1fr`). Type conflicts hide the disclosure. Needs C3. |
| **S3** | Deletions | `4a` ×4, `12a Deletions light` | Severity sorts across the seam: permanent left, recoverable right. No cross-column bulk approve. Typed `DELETE`, case-sensitive, clears on blur, then a full-window confirmation. **The only solid-red fill in the app.** `Keep` is the stronger button in both columns. |
| **S4** | Plan a sync | `5a` ×3 | Nothing that reads zero gets a tile. The destructive item breaks out of the seam into its own band. Gate only when the plan actually deletes. `Run it without the deletion` hidden until G3. Re-check clears an armed gate. |
| **S5** | Activity | `7a` ×4, `6a Activity passes`, `6a Details` | Lookup first — it's the daily question. Never-synced is permanent, warm not crimson. Failed pass expands inline with **the exact daemon string, never paraphrased**. Details panel is where every field the old UI printed as a caption now lives. History block omitted until G1. |
| **S6** | Settings | `8a` ×5 | Every control says what it does to your files, config key in mono beneath. Deletions tab ships via C1; skip-rule counts via C2; the schedule panel ships in its `scan_interval` fallback form until G4. Save-refused must say **"your old settings are still running"**. |
| **S7** | Onboarding | `9a` ×5 | Two steps, not four. CLI check is a silent precondition. Consent comes **after** the merge, and continuous sync does not begin until it's checked. Needs C4 + C5. **Carry `nextOnboardingLatch` forward verbatim, with its test** — onboarding was unreachable on fresh machines until that routing became a pure latch (it holds across the mid-flow config write so writing the folder pair doesn't eject the user to the unreachable screen). This rewrite deletes `state-matrix.js` and rebuilds `app.js`; re-deriving the routing is how that bug comes back. |
| **S8** | Tray | `10a` ×6 | See §6 — this one has real architectural risk. |
| **S9** | Notifications | `11a` ×5 | Four events interrupt, twelve categories stay silent. **No destructive action in any banner, ever.** Coalesce within 30s; never stack two. Needs C6. |
| **S10** | Light theme | `12a` ×7 product + the `12a Tray light` glyph specimen, plus mechanical mapping for the seven undrawn screens | Light is not an inversion — it uses the darker end of each ramp. Surface is `#FAF8F5`, not white. **SVG gradient stops must be theme-aware** (duplicate defs per theme or drive from CSS variables) — easy to miss, and it's the one structural edit light needs beyond the mask colour. |

### Phase 1 — cheap capability work

Sequenced ahead of the screen that needs it.

| Task | Work | Unblocks |
| --- | --- | --- |
| **C1** | `deletion_policy` ↔ `[delete_approval]` mapping in `gui-core/config_io.rs` | S6 tab 3 |
| **C2** | Skip-rule live match counts over the index (`gui-core/index_read.rs`) | S6 tab 2 |
| **C3** | Client-side prose diff summary over `read_conflict_pair` | S2 version cards |
| **C4** | Free-space check on the local root | S7 step 2 |
| **C5** | Distro detection from `/etc/os-release` | S7 CLI-missing |
| **C6** | `notify_policy` (GUI-local) + the four notification triggers | S9 |

### Phase 2 — engine

Separate milestone (**Desktop UI v2 — engine**), separate PRs, each closing a named deviation.

| Task | Work | Frame it completes |
| --- | --- | --- |
| **G1** | Per-path history query (IPC verb + index read) | `7a File lookup` history block |
| **G2** | Byte totals per direction per window (daemon counters) | `2a Syncing` footer line, `7a` seam counts |
| **G3** | Filtered apply — plan minus destructive actions (relates to #100) | `5a Plan` → `Run it without the deletion` |
| **G4** | `full_scan_schedule` config key + daemon scheduler | `8a Settings` schedule panel, `8a Schedule monthly` |
| **G5** | *(optional)* native `deletion_policy` key | the mono key line in `8a Deletions tab` |

---

## 6. Known risks

**The tray panel is not a native menu.** `10-tray.md` replaces the current text menu with the
360px compact panel — hexagon, seam, sentence — with menu rows below. libayatana-appindicator
cannot render that. It needs a **borderless, always-on-top webview window** showing
`ui/compact.js` plus the menu section, with the native menu kept for right-click (KDE convention).
Two sub-risks: the indicator's screen position isn't reliably queryable on GNOME (fall back to the
spec's `top:40px; right:16px`, bottom-right on KDE), and the panel window must not steal focus or
linger after blur. Prototype this before committing S8's estimate.

Also in S8: the five glyphs ship as **symbolic SVGs** so the theme recolours them (today they're
five PNGs), because a tray icon may be forced monochrome — which is why state is carried by
**fill, not hue**.

**Notification action buttons.** `Keep them` / `Review` / `Sign in` require the notification server
to support actions. `tauri-plugin-notification` may not expose them; the fallback is `notify-rust`
or zbus directly. Validate early — the actions are part of the spec, and the *absence* of a Delete
button is a deliberate safety property, not an omission.

**WebKitGTK vs Playwright WebKit.** The harness runs in Playwright; the app runs in WebKitGTK.
Mitigated by asserting computed styles (engine-stable) rather than rasterized pixels — this is a
reason the style gate beats screenshot diffing here, not just a convenience.

**Fitting 1040×764.** Several screens are dense (`8a Settings`, `5a Plan`, `7a File lookup`). The
fit gate catches overflow, but a screen that overflows needs a *design* answer, not a CSS hack —
`02-shell.md` prescribes `overflow-y:auto` with the cut falling on a row boundary. Raise it rather
than shrinking type.

**Fonts may not be fetchable offline.** The v1 foundation build hit exactly this. If `npm` can't
reach the registry, the woff2 files must be vendored by hand before F1 can close — and F1 blocks
every pixel comparison downstream.

**GUI consumes `local_root` verbatim (open issue #135).** A tilde in the settings form still breaks
GUI-side features. S6 touches that form; fold #135 in rather than reproducing the bug in new code.

---

## 7. Delivery

**Milestone: "Desktop UI v2".** Mirrors the v1 shape (milestone #1, F1–F3 + S1–S11), which is the
pattern this repo already reviews well.

**Issues:** one tracking epic; P0.1–P0.3; F1–F9; C1–C6; S1–S10; G1–G5 (Phase 2, separate
milestone). Labels `ui`, plus `design-v2`.

**Branching — stacked on `ui-design`.** Feature branches merge into `ui-design`; `ui-design` opens
one PR to `main` when Phase 1 is complete. Keeps every increment reviewable without shipping a
half-finished redesign to `main`, and the redesign is not incrementally usable — a shell with the
old sidebar and three new screens is worse than either.

**Per PR, per the standing workflow:** commit → push → PR → exhaust Copilot review until it returns
no comments → all CI green → merge → delete the branch both sides.

**Suggested order.** P0 → F1 (fonts/tokens, blocking) → F2+F3 (hexagon+seam) → F8+F9 (harness and
fixtures early, so every screen after this lands against a live gate) → F4–F7 → S1 (proves the
whole stack on the screen with the most states) → C1–C5 → S2, S3, S4, S5, S6, S7 → S8, S9 →
S10 last (it needs every screen to exist before it can mechanically map them).

**CI additions:** the `frontend` job gains the fidelity job (style + copy + fit gates). The Rust
job is unaffected except by C1/C2 and the Phase-2 engine work; keep `--workspace` on clippy and
test or `gui/gui-core` and `gui/src-tauri` are silently skipped.

---

## 8. Acceptance checklist

`14-behaviour-and-state.md` ships one. Marked here by who checks it:

| Check | Gate |
| --- | --- |
| Hexagon pointy-top at every size; no nested circle or ring | harness (SVG `d` + child count) |
| Hexagon does not move between states of the same screen | harness (position asserted per state) |
| Footer's four doors never move or reorder | harness |
| Seam absent when settled; stops above every full-width band | harness |
| Every centred element on the seam has an opaque background mask | harness |
| No colour anywhere on a settled screen | harness (hue scan of computed colours) |
| Keep is the highest-contrast button on Conflicts and Deletions | harness |
| Solid red only on the armed confirmation and permanent-deletion markers | harness (global `#FF3B3B` scan) |
| No destructive action in any notification | review + unit test on the banner builders |
| `Close window` / `Quit` sub-labels present in the tray | unit test |
| Every window fits 1040×764, no clipping, no overflow onto the footer | fit gate |
| Tray glyph distinguishable in one colour at 16px, all five states | review |
| Both themes: contrast checked, gradients theme-aware | harness + a contrast script |
| `prefers-reduced-motion` honoured | harness (assert under emulated preference) |
| Daemon error strings shown verbatim, never paraphrased | review + copy gate |

---

## 9. Open items for the designer

1. **How do you get back to the main screen** from a door route? (§3.3)
2. **`#6D7783` at 4.33:1** — decided: keep as drawn, logged as a known deviation. Recorded here so
   it isn't reopened.
3. **Seven screens have no light frame.** Their light theme is mechanically mapped and cannot be
   compared against a drawn artefact. Flagged, not covered.
4. **Icon set swap.** `01-foundations.md` §8 recommends replacing the Unicode glyphs with a
   1.5px-stroke line set (Lucide or Phosphor Light). Doing it changes every glyph's computed
   geometry and so every affected frame's expectations. Recommend shipping Phase 1 with the
   Unicode glyphs exactly as drawn — that *is* the measured spec — and filing the swap as its own
   issue with its own regenerated frames.
