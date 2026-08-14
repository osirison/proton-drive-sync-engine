# Decisions — design-v2

The build decisions, written once so they are not reopened per screen (**P0.3**, #164). Five were
taken before the build; one of them the build then overturned, and that is recorded here rather than
left as the plan's original assumption.

Each entry names where the decision is **embodied** — the file or commit the tree can be checked
against — as well as where it was originally taken. A decision nobody can check against the tree is
not a decision, it is a preference.

**Where the rest live.** Per-screen departures and everything measured during the build are in
`DEVIATIONS.md`, numbered in resolution order. Two of its sections are product decisions rather than
measurements and are cross-referenced below: **§94** (navigation, 2026-08-13) and **§95**
(`unreachable` was the socket, not Proton, 2026-08-14).

| # | Decision | Embodied at |
| --- | --- | --- |
| 1 | Quiet tier stays `#6D7783` | `gui/src/styles/tokens.css` — `--text-5` |
| 2 | Window fixed at 1040×764 | `gui/src-tauri/tauri.conf.json` — `"resizable": false` |
| 3 | Doc-vs-frame precedence | `DEVIATIONS.md` header + every numbered section |
| 4 | Home is a door (supersedes the plan's assumption) | `gui/src/js/routes.js` — `FOOTER_ORDER`; `gui/src/js/fixtures/fids.js` — `doorKeys` |
| 5 | Ship the Unicode glyphs as drawn | `gui/src/js/ui/` — `bands.js`, `rows.js`, `controls.js`, `dialog.js` |

---

## 1. The quiet tier stays `#6D7783`

**Decided.** `--text-5` ships at the value the prototype draws — 4.33:1 on `#0A0B0D`, under WCAG AA
for small text — as a deliberate quiet tier for mono captions, metadata and timestamps at 10.5–12px.
Not lifted to `#767F8C`.

**Why.** It is the measured value of the drawn frames, not a choice made in the token file. Lifting
it would make the token disagree with every frame that draws a caption.

**Embodied at.** `gui/src/styles/tokens.css` — `--text-5: #6d7783`, with the ratio and the word
`known deviation` in the token's own comment. Light maps it to `#6b7280`; `DEVIATIONS.md` §9
measured that at sixteen of seventeen nodes and named the seventeenth (`#9CA3AF`, one queued-transfer
arrow) a drawing inconsistency rather than a second tier.

**Checkable form.** The token cannot be lifted while the fidelity gate stands, **because**
`assert.mjs` compares `color` exactly after normalisation to `rgb()` (`gui/tools/fidelity/props.mjs`
— colours exact, lengths ±0.5px) at every node the frames draw in that tier; **only** re-drawing the
prototype and regenerating `gui/tools/fidelity/frames/*.json` would let it move.

**What it cost the gate, and where that is written down.** The contrast gate could not be a fixed
WCAG floor because of this decision. `gui/tools/fidelity/check-contrast.mjs` says so in its own
header — a gate at 4.5 would fail frames that _are_ drawn — and is built instead as a conjunction of
an absolute light floor (3.0) and a dark→light parity floor (0.5), which is a rule about mapping a
token to the wrong end of its ramp rather than about absolute legibility.

**Taken at.** `README.md` § Fidelity; `IMPLEMENTATION-PLAN.md` § Scope decisions and §9 item 2.

## 2. The window is fixed at 1040×764

**Decided.** `resizable: false`, and the two minimums (`minWidth: 900`, `minHeight: 600`) dropped.
The dimensions were already right.

**Why.** Every in-scope frame is drawn at one size, and the fit gate is what makes "it fits" a
machine-checkable claim rather than a review opinion.

**Embodied at.** `gui/src-tauri/tauri.conf.json` — `"width": 1040, "height": 764,
"resizable": false`, with no minimums. `gui/tools/fidelity/assert.mjs` opens every frame at exactly
1040×764 and fails a full-window frame whose root `scrollWidth`/`scrollHeight` exceeds it or whose
content paints over the footer.

**Checkable form.** `01-foundations.md` §4's reflow rules — seam stays at 50%, columns stack below
~880px, hairline dropped — cannot be asserted by this harness, **because** no frame is drawn at a
second width, so a stacked layout has nothing to be compared against; **only** a second set of drawn
frames (with `frames/*.json` regenerated from them) would give them a gate. Deferred, and tracked:
**#273**.

**What it costs, measurably.** #221 — `3a Conflicts cleared` is drawn 522×766, so S2 centres a 520px
column inside the fixed window and the footer stays wide; 15 assertions carried in
`gui/tools/fidelity/known-deviations.mjs`, `DEVIATIONS.md` §74.

## 3. Precedence, when the spec disagrees with itself

**Decided.** Fixed order, applied everywhere:

1. the `.md` files are normative for **tokens, rules, semantics and copy**;
2. the matching 1040 / 600 / 520 / 360 frame is normative for that screen's **layout geometry and
   per-element colour**;
3. the illustrative swatches in the prototype's "The system" header block are **not** normative;
4. every conflict is recorded in `DEVIATIONS.md` with its resolution, so nobody re-litigates it
   mid-build.

**Embodied at.** `DEVIATIONS.md` — the rule is restated in its header and every numbered section
resolves under it. `IMPLEMENTATION-PLAN.md` §1.3 is where it was written, with the first nine
conflicts already tabulated.

**Checkable form.** Rule 1 alone cannot separate two tokens that share one dark value, **because**
dark draws them identically and the doc names the shared value once (`#23262D` does four jobs;
`#1A1D22` two, one of which is the window's own 1px edge); **only** a lockstep walk of a dark/light
frame pair attributes each light value to the token it replaces. That method is written up under
`DEVIATIONS.md` § Method, and §8/§8a/§9 are the rows that came out of it.

**Two amendments the build made to the method, both from being wrong first.**

- The walk compares a frame's _descendants_, and a frame is not its own descendant — so the window
  box's own properties were invisible in the first pass and a wrong `--border-subtle` reached the
  light theme (`DEVIATIONS.md` § Method caveat, §8a). Include the root node.
- A `12a` frame's ground truth is the frame's, not the page's: a headless tool that resolves a CSS
  custom property reads the machine's `prefers-color-scheme` unless the theme is pinned
  (`DEVIATIONS.md` §91).

## 4. Getting back to the main screen — the build answered it, and answered it differently

**Assumed, then superseded.** `IMPLEMENTATION-PLAN.md` §3.3 recorded that no frame shows how you
leave a door route, and assumed: clicking the **active** door returns you to root, with the header
app mark as a second home affordance. Flagged for the designer, cheap to change.

**Decided 2026-08-13, against the drawings** (#266, merged `2032549`):

- `FOOTER_ORDER` gained `main` at its head, labelled **Home**. The lit door is a no-op.
- The app mark stays a home affordance as well — a second route home is redundant, not wrong.
- The four doors are drawn on **every** screen but the onboarding takeover, under the action bar
  where there is one. On a fresh machine there is nowhere to navigate to.

**Why the assumption failed.** Two things, both in `DEVIATIONS.md` §94. A tab that toggles is not a
tab — clicking `Activity` while on Activity left the screen you were looking at. And it was silently
destructive: re-entering a screen re-renders it, so the toggle discarded a half-typed lookup and an
in-flight rehearsal.

**Embodied at.** `gui/src/js/routes.js` — `ROUTES.main` (`kind: "root"`, `label: "Home"`) and
`FOOTER_ORDER = ["main", "activity", "plan", "settings", "details"]`, both carrying the dated
decision in comments. `gui/src/js/fixtures/fids.js` — `doorKeys` maps the app's door _i_ to the
frame's `span[i-1]` and answers `null` for door 0.

**Checkable form.** The five-door footer cannot be reconciled with the frames by the fidelity gate,
**because** the frames draw four doors and light none of them on the main screen while the app draws
five and lights Home; **only** the `doorKeys` offset keeps the four drawn doors comparable and leaves
the undrawn one out of the comparison instead of matching it to a node that does not exist. The four
resulting departures are tagged `decision` in `gui/tools/fidelity/known-deviations.mjs` — a third
class beside `structural`, for a departure no capability closes and no issue tracks, carrying the
must-still-fail clause unchanged.

## 5. Ship Phase 1 with the Unicode glyphs exactly as drawn

**Decided.** The prototype's Unicode glyphs are the shipped icon set. The 1.5px-stroke line set that
`01-foundations.md` §8 recommends (Lucide or Phosphor Light) is not adopted in Phase 1.

**Why.** The drawn glyph _is_ the measured spec — its box, `font-size` and colour are what the frames
assert. Swapping the set changes the spec, not the implementation of it.

**Embodied at.** `gui/src/js/ui/bands.js` — `warnGlyph(glyph = "⊘")`; `gui/src/js/ui/rows.js` — the
13px mark slot shared by `→ ← ＋ ↷`; `gui/src/js/ui/controls.js` — the per-instance glyph tone;
`gui/src/js/ui/dialog.js` — the `✕`. `gui/package.json` carries no icon dependency (the only asset
dependencies are the two `@fontsource` typefaces from F1).

**Checkable form.** The swap cannot be made under the current gate, **because** a line-set icon
replaces a text node with an `<svg>`, so every `data-fid` mapped to a glyph would compare against a
node the frames do not contain and the glyph's computed box stops being the property that describes
it; **only** re-drawing the prototype with the new set and regenerating `frames/*.json` closes it.
Deferred, and tracked: **#272**.

**Taken at.** `IMPLEMENTATION-PLAN.md` §9 item 4; `README.md` § Assets records the same
recommendation without deciding it.
