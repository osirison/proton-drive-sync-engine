---
trigger: header spacer, header/span[1], 731.08, "22 frames pin", pill drawn unconditionally, boxComparability, R-N1, check-n1-identity
depends_on: gui/tools/fidelity/props.mjs, gui/tools/fidelity/assert.mjs, gui/tools/fidelity/check-n1-identity.mjs
recorded: 2026-10-09
---

# The 22 frames that "pin the header spacer" pin nothing the style gate asserts

**The claim in the brief and the ADR:** 22 of the frames record the header's `flex:1` spacer at a fixed width
(`header/span[1]` `w: 731.08` on `2a Settled`), so a node added to the header at one folder would fail
`assert.mjs` with `box.w 731.08 vs <less>`.

**What was measured (2026-10-09, #102 phase 5c-1):** the folder pill was drawn unconditionally (the selector
returned its props at any folder count) and `assert.mjs` still reported `2/2 frames mapped, 4411 assertions,
0 failures` on `2a Settled` and `2a Syncing`. Two reasons, both structural:

1. The header holds the `⋯` glyph, which is not in the bundled faces. `boxComparability` takes every node whose
   PARENT's subtree holds an unbundled glyph out of the box comparison, so the spacer's width is recorded and
   asserted nowhere.
2. An extra node no fixture names is stamped by nothing, and the style gate walks stamped nodes.

`check-n1-identity.mjs` does not see it either when the pill is drawn in BOTH renderings it compares (legacy
reply and one-pair reply): equal bytes.

**What catches it now:** `check-n1-identity.mjs` states the absence outright (`class="pair-select"` must not
appear in any frame that lists no more than one folder, with or without `?pairs=1`, and must appear in the
frames that list two), and `fidelity:pairs` scenario 16 asserts no `.pair-select` at one folder. Poison either
by returning the selector's props at `pairs.length < 1`.

**Do not rely on** "the spacer box fails" as the proof that D2 holds for a new header node.
