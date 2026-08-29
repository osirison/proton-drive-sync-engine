// The assertions a screen CANNOT pass yet, each named with the issue that closes it — or, for the
// five marked `structural`, with the reason nothing ever will.
//
// WHY THIS FILE EXISTS, AND WHY IT IS NOT AN ESCAPE HATCH.
//
// `IMPLEMENTATION-PLAN.md` §4 splits the design's ten assumed capabilities into six the GUI can do
// now and four that are engine work, and it says what Phase 1 does about the rest: **omit the
// clause, never fake it.** So `2a Settled` draws `last synced 2 minutes ago · 12,480 files · 41.2 GB`
// and Phase 1 renders the first third of it, because no command reports an index-wide file count or
// byte total. That is the documented plan working correctly — and it lands in the style gate as a
// node 195px wide where the frame has one 390px wide.
//
// Three ways out, and only one of them is honest:
//
//   1. Fill the clause with plausible numbers. Forbidden outright — a screen built against invented
//      data is how a Phase-2 design gets settled by a preview fixture (DEVIATIONS §60).
//   2. Leave the gate red. A gate that is expected to fail is a gate nobody reads, and the next real
//      regression arrives in a build that was already failing.
//   3. Record the exact assertion, with the issue that closes it, and require it to still be failing.
//
// The third is what this is, and the last clause is what keeps it from rotting into a mute list:
// **an entry that no longer fails is itself a failure.** The day #207 lands and the settled sub-line
// grows its two clauses, this file fails the build until the row is deleted. An allow-list that can
// only ever grow is a list of things nobody will remove.
//
// The bar for adding a row: the difference must be a MISSING CAPABILITY, already written up in
// `docs/design-v2/DEVIATIONS.md` with an open issue against it. A colour that is wrong, a padding
// that is off, a node that was never mapped — none of those belong here. They are bugs.
//
// The one exception is `structural`, and it is narrow on purpose: the property cannot be carried by
// any element the app draws, so there is no capability to wait for. See the tag's own note below —
// and note that the must-still-fail clause applies to it unchanged, which is what keeps "structural"
// from becoming the escape hatch this file's first paragraph says it is not.

/**
 * @property frame  the `data-screen-label` exactly as `frames/index.json` carries it
 * @property key    the node key, or `(fit)` for a fit-gate row
 * @property props  which assertions on that node are expected to fail — never a wildcard, so a
 *                  SECOND thing going wrong on the same node is still caught
 * @property detail the EXACT mismatch, verbatim as `assert.mjs` formats it. Absorbs that one
 *                  difference and nothing else; see the note below.
 * @property issue  the issue that closes it — unless `structural`, which nothing closes
 * @property structural
 *                  the app CANNOT EXPRESS this property, so no capability lands and no issue
 *                  closes it. The five `10a In situ` offsets are the case it was added for: the
 *                  drawn panel is placed on a desktop mock with `position:absolute; top; right`,
 *                  and the shipped one is a borderless window whose viewport IS the panel, so the
 *                  offsets live on the window and there is no element to compare. Filing an issue
 *                  for it would be filing an issue against the architecture. Everything else about
 *                  a row is unchanged — it must still name the exact difference, and it must still
 *                  FAIL: a structural row that stops failing is as much of a lie as any other.
 * @property decision
 *                  THE PRODUCT CHOSE AGAINST THE DRAWING, so no capability is missing and no issue
 *                  closes it either. The four `§94` rows are the case it was added for: the doors
 *                  are drawn on every screen, which takes 50px from a content region the frames
 *                  measured without them. Narrower than it looks — it may only be used for a
 *                  departure recorded in DEVIATIONS.md with the decision and its date, never for
 *                  "the app does it differently". Must still fail, like every other row.
 * @property why    one line, in the same voice as DEVIATIONS.md
 */
export const KNOWN_DEVIATIONS = [
  // ---- S9 · the three banners inside the desktop mock ----
  //
  // ONE SENTENCE IS SHORTER THAN THE DRAWN ONE, and every row below is it wrapping to one line
  // instead of two — the banner, its head, the text column and the sentence itself, four nodes for
  // the one cause. Not a layout difference: `renderBanner` draws exactly what the frame draws, at
  // the length Phase 1 can honestly write. (The deletion banner's four rows are gone: it counts the
  // subtree now, #208.)
  {
    frame: "11a In situ",
    key: "div[1]/div/div[2]",
    props: ["box.h"],
    detail: "109.5 vs 90.75",
    issue: "#207",
    why: "`12,480 files, 41.2 GB` is the index-wide total (G7 #207), and the rule for a missing capability is to drop the clause rather than fill it — so the body is one line where the frame draws two",
  },
  {
    frame: "11a In situ",
    key: "div[1]/div/div[2]/div",
    props: ["box.h"],
    detail: "77.5 vs 58.75",
    issue: "#207",
    why: "`12,480 files, 41.2 GB` is the index-wide total (G7 #207), and the rule for a missing capability is to drop the clause rather than fill it — so the body is one line where the frame draws two",
  },
  {
    frame: "11a In situ",
    key: "div[1]/div/div[2]/div/div",
    props: ["box.h"],
    detail: "77.5 vs 58.75",
    issue: "#207",
    why: "`12,480 files, 41.2 GB` is the index-wide total (G7 #207), and the rule for a missing capability is to drop the clause rather than fill it — so the body is one line where the frame draws two",
  },
  {
    frame: "11a In situ",
    key: "div[1]/div/div[2]/div/div/div[2]",
    props: ["box.h"],
    detail: "37.5 vs 18.75",
    issue: "#207",
    why: "`12,480 files, 41.2 GB` is the index-wide total (G7 #207), and the rule for a missing capability is to drop the clause rather than fill it — so the body is one line where the frame draws two",
  },
  // ---- S8 · the tray panel, drawn floating over a desktop ----
  //
  // `10a In situ` is the only frame that draws the panel WHERE IT ACTUALLY LIVES: over a wallpaper,
  // not on the prototype's own page. Two facts follow from that, and neither is reproducible by the
  // panel's DOM, because in the shipped app the panel IS the window.
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["position"],
    detail: "absolute vs static",
    structural: true,
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. MEASURED on a live Plasma 6.7 session (S8 verification): a real pointer click on the indicator arrived as `Activate(3132, 2112)` and the panel opened centred on it, its bottom edge on `_NET_WORKAREA`'s at 2068 — clamped into the work area and therefore UPWARD, which is what the spec's fixed `top:40px` gets wrong on every bottom-panel desktop. Re-measured at all five panel heights and every one of them anchored there. The design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["top"],
    detail: "40px vs auto",
    structural: true,
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. MEASURED on a live Plasma 6.7 session (S8 verification): a real pointer click on the indicator arrived as `Activate(3132, 2112)` and the panel opened centred on it, its bottom edge on `_NET_WORKAREA`'s at 2068 — clamped into the work area and therefore UPWARD, which is what the spec's fixed `top:40px` gets wrong on every bottom-panel desktop. Re-measured at all five panel heights and every one of them anchored there. The design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["right"],
    detail: "16px vs auto",
    structural: true,
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. MEASURED on a live Plasma 6.7 session (S8 verification): a real pointer click on the indicator arrived as `Activate(3132, 2112)` and the panel opened centred on it, its bottom edge on `_NET_WORKAREA`'s at 2068 — clamped into the work area and therefore UPWARD, which is what the spec's fixed `top:40px` gets wrong on every bottom-panel desktop. Re-measured at all five panel heights and every one of them anchored there. The design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["bottom"],
    detail: "38.5px vs auto",
    structural: true,
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. MEASURED on a live Plasma 6.7 session (S8 verification): a real pointer click on the indicator arrived as `Activate(3132, 2112)` and the panel opened centred on it, its bottom edge on `_NET_WORKAREA`'s at 2068 — clamped into the work area and therefore UPWARD, which is what the spec's fixed `top:40px` gets wrong on every bottom-panel desktop. Re-measured at all five panel heights and every one of them anchored there. The design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["left"],
    detail: "662px vs auto",
    structural: true,
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. MEASURED on a live Plasma 6.7 session (S8 verification): a real pointer click on the indicator arrived as `Activate(3132, 2112)` and the panel opened centred on it, its bottom edge on `_NET_WORKAREA`'s at 2068 — clamped into the work area and therefore UPWARD, which is what the spec's fixed `top:40px` gets wrong on every bottom-panel desktop. Re-measured at all five panel heights and every one of them anchored there. The design's intent is delivered by the thing that can hold it",
  },

  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
    detail: "rgba(255, 255, 255, 0.1) vs rgba(255, 107, 107, 0.3)",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17: the tray form now takes `10-tray.md`'s desktop-facing edge (`.compact-panel.is-tray`, compact.css), and `.compact-panel.is-attention` still overrides it — the same reasoning DEVIATIONS §58d used to keep it, restated rather than reversed. This ONE frame is drawn in the needsYou state, so is-attention applies here too, and the built panel therefore still shows the attention edge (`rgba(255,107,107,.3)`) against the frozen capture's desktop edge (`rgba(255,255,255,.1)`) — the decision was explicit that the attention edge must not be disturbed, so this row is not something the code change could retire. The four standalone `10a` panels are none of them needsYou; they took the tray edge cleanly and are recorded as new rows of their own instead. DEVIATIONS §101",
  },

  // The other side of the same decision: none of these four is drawn needsYou, so `.is-attention`
  // never enters it and each now cleanly takes the tray's desktop edge, disagreeing with its own
  // frame — which still draws `#23262D` like every other compact panel, since #261 settled the
  // BUILD's edge and not a redraw of these four (the decision's own "ideally redrawn instead" is
  // future design work, not this commit's). `key: ""` is the frame's own root node — these four
  // fixtures draw the panel bare, with no desktop-mock wrapper the way `10a In situ` has one.
  {
    frame: "10a Settled",
    key: "",
    props: ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
    detail: "rgb(35, 38, 45) vs rgba(255, 255, 255, 0.1)",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17: the tray form takes `10-tray.md`'s desktop-facing edge instead of `--border-chrome`. This frame is not drawn needsYou, so nothing overrides it, and the frame itself is unchanged — recorded rather than redrawn, per the decision. DEVIATIONS §101",
  },
  {
    frame: "10a Syncing",
    key: "",
    props: ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
    detail: "rgb(35, 38, 45) vs rgba(255, 255, 255, 0.1)",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17: the tray form takes `10-tray.md`'s desktop-facing edge instead of `--border-chrome`. This frame is not drawn needsYou, so nothing overrides it, and the frame itself is unchanged — recorded rather than redrawn, per the decision. DEVIATIONS §101",
  },
  {
    frame: "10a Offline",
    key: "",
    props: ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
    detail: "rgb(35, 38, 45) vs rgba(255, 255, 255, 0.1)",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17: the tray form takes `10-tray.md`'s desktop-facing edge instead of `--border-chrome`. This frame is not drawn needsYou, so nothing overrides it, and the frame itself is unchanged — recorded rather than redrawn, per the decision. DEVIATIONS §101",
  },
  {
    frame: "10a Paused",
    key: "",
    props: ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
    detail: "rgb(35, 38, 45) vs rgba(255, 255, 255, 0.1)",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17: the tray form takes `10-tray.md`'s desktop-facing edge instead of `--border-chrome`. This frame is not drawn needsYou, so nothing overrides it, and the frame itself is unchanged — recorded rather than redrawn, per the decision. DEVIATIONS §101",
  },

  {
    frame: "2a Settled",
    key: "div[0]/div[2]",
    props: ["box.w"],
    detail: "390.02 vs 195.02",
    issue: "#207",
    why: "the settled sub-line's `· 12,480 files · 41.2 GB` is G7 — no command reports index-wide totals, so Phase 1 draws `last synced 2 minutes ago` alone and the line measures 195px against the drawn 390px",
  },

  {
    frame: "3a Conflict",
    key: "div[1]/div[1]/div[1]",
    props: ["box.w"],
    detail: "324.7 vs 145.31",
    issue: "#217",
    why: "the meta line's `· last agreed 3 hours ago` needs the baseline's timestamp, and `FileRecord` has no last-synced field — the daemon's conflict arm overwrites the original's record with the CURRENT local state, so even the mtime proxy is gone. Phase 1 draws `a plain text file` alone, at 145px against the drawn 325px",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[0]",
    props: ["margin-right"],
    detail: "0px vs 260px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself for this state — S2 draws the body as a centred 520px column inside the fixed, non-resizable 1040 window, and a 520 column in a 1040 window has 260px either side where the frame's own 522px window has none. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[0]",
    props: ["margin-left"],
    detail: "0px vs 260px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself for this state — S2 draws the body as a centred 520px column inside the fixed, non-resizable 1040 window, and a 520 column in a 1040 window has 260px either side where the frame's own 522px window has none. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]",
    props: ["padding-right"],
    detail: "24px vs 40px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]",
    props: ["padding-left"],
    detail: "24px vs 40px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div",
    props: ["gap"],
    detail: "22px vs 34px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div",
    props: ["box.w"],
    detail: "472 vs 960",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div",
    props: ["box.h"],
    detail: "31 vs 32",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[0]",
    props: ["box.w"],
    detail: "43.86 vs 45.61",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[0]",
    props: ["box.h"],
    detail: "15 vs 16",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[1]",
    props: ["box.w"],
    detail: "63.59 vs 66.14",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[1]",
    props: ["box.h"],
    detail: "15 vs 16",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[2]",
    props: ["box.w"],
    detail: "48.19 vs 50.11",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[2]",
    props: ["box.h"],
    detail: "15 vs 16",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[3]",
    props: ["box.w"],
    detail: "39.63 vs 41.2",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[3]",
    props: ["box.h"],
    detail: "15 vs 16",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221): the shell never resizes itself, so the footer — a child of the WINDOW, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame. DEVIATIONS §103",
  },

  // ---- S3 · the deletions screen. One row for one word, and three for one window size.
  //
  // THE COUNT LANDED AND THE NOUN CANNOT. #208 gave the card its subtree aggregate, so the
  // consequence wraps to the drawn two lines, the facts strip is built and every height matches.
  // What is left is a single word: the frame says `1,204 photos` because its folder is a photo
  // library, and no engine can know that — `subtree_files` counts FILES. A UI that guessed the noun
  // from the extensions inside would be inventing a fact on the screen whose whole job is not to,
  // so the app says what is true and the drawing keeps its word. Decided 2026-08-15, DEVIATIONS §75.
  {
    frame: "4a Deletions",
    key: "div[1]/div[1]/div[0]/div[2]/div[1]/strong",
    props: ["box.w"],
    detail: "122.27 vs 104.56",
    decision: true,
    why: "the emphasised loss is the aggregate itself, and `1,204 files, 8.4 GB` is 17.71px narrower than the drawn `1,204 photos, 8.4 GB` — the same number, the noun the engine can stand behind",
  },
  {
    frame: "4a Armed",
    key: "div[0]/div[0]",
    props: ["box.w"],
    detail: "507.73 vs 470.66",
    decision: true,
    why: "`Delete 1,204 files from this computer?` against the drawn `1,204 photos` — the same word as the card, in the question. Phase 1's `Delete photos/2019 …` happened to measure within tolerance and this does not, which is the coincidence DEVIATIONS §75 recorded as one",
  },
  {
    frame: "4a Empty",
    key: "div",
    props: ["margin-left"],
    detail: "0px vs 260px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the shell never resizes itself for this state either — S3 draws the body as a centred 520px column inside the fixed, non-resizable 1040 window, and a 520 column in a 1040 window has 260px either side where the frame's own 522px window has none. DEVIATIONS §103",
  },
  {
    frame: "4a Empty",
    key: "div",
    props: ["margin-right"],
    detail: "0px vs 260px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the shell never resizes itself for this state either — S3 draws the body as a centred 520px column inside the fixed, non-resizable 1040 window, and a 520 column in a 1040 window has 260px either side where the frame's own 522px window has none. DEVIATIONS §103",
  },
  {
    frame: "4a Empty",
    key: "div",
    props: ["box.h"],
    detail: "420 vs 662",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the block is `flex: 1` between a 52px header and a 50px footer the frame does not draw, so it fills 662 of the fixed 1040×764 window where the frame's whole surface is 422 — pinning it to 420 would leave the empty state floating in the top half of a window whose remaining space belongs to nothing. DEVIATIONS §103",
  },

  // ---- S4 · the plan screen. Three causes: four rows for the byte totals nothing reports (#191),
  // four for two buttons that need a capability the daemon does not have (#192), and eight —
  // `sixteen` before the door rows below were retired with the doors themselves — for a 522px
  // frame drawn inside the shell's fixed 1040px window. #221 settled `decision: true`, DEVIATIONS
  // §103: the shell never resizes itself, on this frame no less than on `3a Conflicts cleared`.
  {
    frame: "5a Plan",
    key: "div[1]/div[1]/div[0]/div[1]/span[1]",
    props: ["box.w"],
    detail: "71.92 vs 26.22",
    issue: "#191",
    why: "the leaving side's `files, 4.1 MB` loses its byte total: G2 — `PlannedAction` carries no size field, so the dry-run surface has no per-direction total to draw and Phase 1 says `files` alone",
  },
  {
    frame: "5a Plan",
    key: "div[1]/div[1]/div[1]/div[1]/span[1]",
    props: ["box.w"],
    detail: "75.11 vs 26.22",
    issue: "#191",
    why: "the arriving side's `files, 2.6 MB`, absent for the same reason as the leaving side's — one missing field, drawn twice",
  },
  {
    frame: "5a Plan",
    key: "div[2]/div/div",
    props: ["box.w"],
    detail: "756.3 vs 884",
    issue: "#224",
    why: "the destructive band's body takes the width `Leave it alone` would have occupied — that button reads either as the filtered apply (landed as #192's `Run it without the deletion`, drawn in the footer) or as a durable refusal of this one deletion (#224, which the Deletions screen owns), and `06-plan.md` says to hide a button rather than fake whichever the reader assumed",
  },
  {
    frame: "5a Plan",
    key: "div[2]/div/div/div[0]",
    props: ["box.w"],
    detail: "756.3 vs 884",
    issue: "#224",
    why: "the band's title, widened by the same absent button — the body is `flex: 1` and its children fill it",
  },
  {
    frame: "5a Plan",
    key: "div[2]/div/div/div[1]",
    props: ["box.w"],
    detail: "756.3 vs 884",
    issue: "#224",
    why: "the band's consequence sentence, widened by the same absent button; it still wraps to one line, so only the box moves",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[0]/div[1]/span[1]",
    props: ["box.w"],
    detail: "71.92 vs 26.22",
    issue: "#191",
    why: "the leaving side's byte total, absent for the same reason on the safe screen; its per-file sizes (`1.2 MB`, `2.8 MB`, `96 KB`) are absent too and cost no assertion, because each of those rows sits inside a subtree containing an unbundled glyph and the harness does not compare their boxes",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[1]/div[1]/span[1]",
    props: ["box.w"],
    detail: "75.11 vs 26.22",
    issue: "#191",
    why: "the arriving side's byte total, likewise",
  },

  // The 522px window inside a 1040px shell — `3a Conflicts cleared` and `4a Empty`'s situation for a
  // third time, and the first with a seam in it.
  {
    frame: "5a Checking",
    key: "div[0]",
    props: ["margin-left"],
    detail: "0px vs 260px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the shell never resizes itself for this state either — S4 draws the body as a centred 520px column inside the fixed, non-resizable 1040, and that column has 260px either side where the frame's own 522px window has none. DEVIATIONS §103",
  },
  {
    frame: "5a Checking",
    key: "div[0]",
    props: ["margin-right"],
    detail: "0px vs 260px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the shell never resizes itself for this state either — S4 draws the body as a centred 520px column inside the fixed, non-resizable 1040, and that column has 260px either side where the frame's own 522px window has none. DEVIATIONS §103",
  },
  {
    frame: "5a Checking",
    key: "div[0]/div[0]",
    props: ["box.h"],
    detail: "543 vs 542",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the seam is pinned 60px off each end of its block, so it is exactly as tall as the block minus 120 — and the block is one pixel shorter here because the fixed 1040 footer beneath it is one pixel taller than the frame's own 520-window footer (31px of doors against 32). DEVIATIONS §103",
  },
  {
    frame: "5a Checking",
    key: "div[1]",
    props: ["padding-right"],
    detail: "24px vs 40px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the shell never resizes itself, so the footer — a child of the window, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. DEVIATIONS §103",
  },
  {
    frame: "5a Checking",
    key: "div[1]",
    props: ["padding-left"],
    detail: "24px vs 40px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the shell never resizes itself, so the footer — a child of the window, not the body — stays 1040 wide while the frame's is 520; the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. DEVIATIONS §103",
  },
  {
    frame: "5a Checking",
    key: "div[1]/div",
    props: ["gap"],
    detail: "22px vs 34px",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the narrow window's doors sit 22px apart against the wide window's 34 — the same per-width footer metrics, on the same footer node, that the shell's fixed 1040 keeps this frame from ever matching. DEVIATIONS §103",
  },
  {
    frame: "5a Checking",
    key: "div[1]/div",
    props: ["box.w"],
    detail: "472 vs 960",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): the door bar spans its window: the shell's fixed 1040 less 32px either side here, the frame's own 520 less 24 there. DEVIATIONS §103",
  },
  {
    frame: "5a Checking",
    key: "div[1]/div",
    props: ["box.h"],
    detail: "31 vs 32",
    decision: true,
    why: "MAINTAINER DECISION, 2026-08-17 (#221, precedent set by §103): one pixel of line box, from the 13px labels the shell's fixed window draws against the frame's own narrower one's 12.5px. DEVIATIONS §103",
  },
  // S5. Six rows, three gaps, and each is a block the screen omits WHOLE rather than fills.
  {
    frame: "7a Activity quiet",
    key: "div[2]/div[0]",
    props: ["box.h"],
    detail: "172 vs 131",
    issue: "#207",
    why: "the seam's two sides draw `12,480` over `files · 41.2 GB` and no command reports an index-wide count, so Phase 1 omits both numeral rows — the sides lose 41px and the seam line is measured against the block it runs down",
  },
  {
    frame: "7a Activity quiet",
    key: "div[2]/div[2]",
    props: ["box.h"],
    detail: "77 vs 36",
    issue: "#207",
    why: "the two-column grid holding both counted sides, 41px shorter without the numeral rows",
  },
  {
    frame: "7a Activity quiet",
    key: "div[2]/div[2]/div[0]",
    props: ["box.h"],
    detail: "77 vs 36",
    issue: "#207",
    why: "`This computer` keeps its eyebrow and its `watched continuously · checked 2m ago` sub-line, both sourced, and loses only the count between them",
  },
  {
    frame: "7a Activity quiet",
    key: "div[2]/div[2]/div[1]",
    props: ["box.h"],
    detail: "77 vs 36",
    issue: "#207",
    why: "`Proton Drive` loses the same count — and its sub-line too, but to a different gap: `next full check in 4m` is a COUNTDOWN, and while #193 gave the daemon a `full_scan_schedule`, no reply carries the moment it next fires — the schedule exists, the next-due instant is not on any wire. Deriving one from `scan_interval` would still contradict `6a Details`, which draws that interval as its own row",
  },

  // ---- S6 · settings ----
  //
  // EIGHT ROWS, THREE CAUSES, AND EVERY ONE OF THEM IS A SENTENCE THAT IS NOT DRAWN.
  // Nothing here is a colour or a spacing that came out wrong: the tab bodies, the panels, the rule
  // rows and the three radio cards all match the frames exactly. What is left is the shape the
  // omissions leave behind — a helper one line shorter, and a tail that sits lower because the
  // block above it was never built.
  //
  // WAS ELEVEN ROWS AND FOUR CAUSES. #193 built the schedule, so the panel head's three `box.w`
  // rows — the text block, its title and its sub-line all taking the width the missing
  // Weekly/Monthly control was holding — are retired rather than left to pass silently. The control
  // that replaces them is DECLARED (`scheduleMode` in `fids.js`), not merely present: a retirement
  // and a fresh divergence cancel each other out, and an undeclared replacement would take three
  // checked nodes off the books and put nothing back.
  {
    frame: "8a Settings",
    key: "div[2]/div[2]/div[0]",
    props: ["box.h"],
    detail: "109 vs 91",
    issue: "#207",
    why: "the local side's helper drops `12,480 files, 41.2 GB in here today` (G7 — no command reports the folder's totals, and `considered_files` is a count with no byte twin) and keeps the sentence that matters, `Changing it starts a fresh merge`; one line instead of two takes 18px off the column",
  },
  {
    frame: "8a Settings",
    key: "div[2]/div[2]/div[0]/div[2]",
    props: ["box.h"],
    detail: "36 vs 18",
    issue: "#207",
    why: "the helper itself: the drawn sentence wraps to two lines at 458px and the Phase-1 half fits on one",
  },
  {
    frame: "8a Settings",
    key: "div[2]/div[2]/div[1]",
    props: ["box.h"],
    detail: "109 vs 91",
    issue: "#207",
    why: "the Proton side is a grid cell, so it stretches to whatever the taller column is — it loses the same 18px its neighbour does, and its own helper is unchanged",
  },
  // ---- §63b · the split bar's fills are computed, and the frame's two are not ----
  //
  // `9a First sync` paints the sent fill 48px and the received fill 88px of a 400px track, and
  // labels them `44 sent` and `115 received` in the same block. 48:88 is 0.55 and 44:115 is 0.38;
  // no denominator produces both, so the drawing disagrees with its own numbers. Its TOTAL is
  // right — 48+88 = 136 of 400 is 34%, against the 159/471 the line above reads — which is what
  // identifies the split as the hand-drawn half.
  //
  // #243 landed the two counts, so the app computes each fill as its count over `action_total`:
  // 9.3% and 24.4%, the same 33.8% total. Reproducing 48:88 would mean drawing a split that
  // contradicts the labels beside it. Decided 2026-08-16, DEVIATIONS §63b.
  {
    frame: "9a First sync",
    key: "div[0]/div[5]/div[0]/div[0]",
    props: ["box.w"],
    detail: "48 vs 37.36",
    decision: true,
    why: "the sent fill is `uploaded_files / action_total` = 44/471 of 400px, where the frame paints a 48px fill it also labels `44 sent`",
  },
  {
    frame: "9a First sync",
    key: "div[0]/div[5]/div[0]/div[1]",
    props: ["box.w"],
    detail: "88 vs 97.66",
    decision: true,
    why: "the received fill is `downloaded_files / action_total` = 115/471 of the same track — the other half of one hand-drawn split",
  },
  // ---- §9 · one node against sixteen, and it is now drawn ----
  //
  // The queued row's arrow in light. §9 measured it years of frames ago: `#6D7783` maps to
  // `#6B7280` at sixteen nodes and to `#9CA3AF` at exactly this one, "a drawing inconsistency, not
  // a tier", so `--text-5` is the value and `rows.css` has said so at `.transfer-queued
  // .transfer-arrow` ever since. Nothing reached the node until #211 gave the wire a queued row to
  // draw; the decision is §9's and its date is that measurement's.
  {
    frame: "12a Syncing light",
    key: "div[1]/div[0]/div[1]/span[2]",
    props: ["color", "border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
    detail: "rgb(156, 163, 175) vs rgb(107, 114, 128)",
    decision: true,
    why: "the queued arrow is `--text-5`, which light resolves to #6B7280 — §9 measured #9CA3AF at this one node against sixteen others and settled it as a drawing inconsistency rather than a tier",
  },
  // ---- §94 · the doors are drawn on every screen ----
  //
  // The 2026-08-13 decision (DEVIATIONS §94) draws the footer nav under the action bar, so Settings
  // and Plan hand 50px of their content region to navigation. Every row below is that 50px arriving
  // somewhere measurable. No capability closes them; a re-drawn frame would.
  {
    frame: "8a Settings",
    key: "div[2]",
    props: ["overflow"],
    detail: "visible vs auto",
    decision: true,
    why: "the folders tab is 491px of content in a 447px region once the doors take their 50px, and a `flex:1` item with no `min-height:0` refuses to shrink below its content — the window scrolled as a whole and took the footer off the screen. Scrolling the region loses nothing; clipping it would",
  },
  {
    frame: "8a Settings",
    key: "div[2]/div[0]",
    props: ["bottom"],
    detail: "382px vs 332px",
    decision: true,
    why: "the folder pair's seam is pinned 60px off each end of the tab, so its used `bottom` is whatever the tab's height leaves — exactly 50px less with the doors under the bar",
  },
  {
    frame: "8a Skip rules",
    key: "div[2]",
    props: ["overflow"],
    detail: "visible vs auto",
    decision: true,
    why: "the same region on the same screen, and the same 50px: `.settings-content` is one rule for all five tabs",
  },
  {
    frame: "8a Skip rules",
    key: "div[2]/div[2]",
    props: ["margin-top"],
    detail: "85.8125px vs 35.8125px",
    decision: true,
    why: "the same 50px again, arriving as a number rather than as an overflow: the tail is pushed down with `margin-top:auto`, so its used value IS the slack left in the region — and the doors have taken exactly 50px of it. It read 106.812px until #232 built the unsyncable panel above it, which is the 71px the frame draws and the app now does too",
  },
  {
    frame: "5a Plan",
    key: "div[3]/div[1]",
    props: ["overflow"],
    detail: "hidden vs hidden auto",
    decision: true,
    why: "the frame draws nine 33px rows in a 319px block; with the doors under the bar the block is 272px and eight fit, so the list scrolls rather than hiding the ninth action behind a clip the user cannot reach",
  },
  // ---- §96 · the safe body's side lists are bounded by the window ----
  //
  // The other half of the row above, on the other body (#267, DEVIATIONS §96). `5a Plan safe` draws
  // three files and two, so the frame had no occasion to bound them; a seventh file leaving put
  // `Run this sync` 33px below a window that cannot be resized, and 44 put it 1254px below. Seven
  // rows, one shrinking block: the frame's `flex: none` seam block becomes the block that gives, its
  // two columns become flex columns so the squeeze lands on the list rather than on the count above
  // it, and each list scrolls. No capability closes them; a re-drawn frame would.
  {
    frame: "5a Plan safe",
    key: "div[1]",
    props: ["flex-shrink"],
    detail: "0 vs 1",
    decision: true,
    why: "the seam block is the block with the slack — the hero above it is a fixed 300px and the spacer below it has already collapsed by the seventh file — so it is the one that shrinks when the lists outgrow the window",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[0]",
    props: ["display"],
    detail: "block vs flex",
    decision: true,
    why: "the leaving column is a flex column so that a bounded height reaches the list alone: the eyebrow and the 42px count are floored by their own content, and the rows are what is left to give",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[0]",
    props: ["flex-direction"],
    detail: "row vs column",
    decision: true,
    why: "the same column, stacking the way it already drew — `flex-direction` is only observable at all because the display changed",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[0]/div[2]",
    props: ["overflow"],
    detail: "visible vs auto",
    decision: true,
    why: "six 33px rows fit under a 300px hero and the seventh does not; the list scrolls rather than pushing the primary action off a window with no scrollbar of its own — 02-shell.md's own rule for a list that can genuinely exceed its space",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[1]",
    props: ["display"],
    detail: "block vs flex",
    decision: true,
    why: "the arriving column, for the same reason and by the same rule: one stylesheet rule covers both sides of the seam",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[1]",
    props: ["flex-direction"],
    detail: "row vs column",
    decision: true,
    why: "likewise",
  },
  {
    frame: "5a Plan safe",
    key: "div[1]/div[1]/div[1]/div[2]",
    props: ["overflow"],
    detail: "visible vs auto",
    decision: true,
    why: "the arriving list scrolls too — the taller side is whichever way the plan happens to run, and a rule that bounded only the leaving one would be a fix for half the plans",
  },
  {
    frame: "8a Save refused",
    key: "div",
    props: ["box.h"],
    detail: "165 vs 145",
    issue: "#236",
    why: "the refusal's body drops `Create the folder on Proton Drive first, or pick a different one.` — `write_config` validates TOML and never contacts Proton Drive, so it cannot know a remote folder is missing (G22) — and one line instead of two takes 20px off the dialog",
  },
  {
    frame: "8a Save refused",
    key: "div/div",
    props: ["box.h"],
    detail: "165 vs 145",
    issue: "#236",
    why: "the text column inside it, losing the same line",
  },
  {
    frame: "8a Save refused",
    key: "div/div/div[1]",
    props: ["box.h"],
    detail: "40 vs 20",
    issue: "#236",
    why: "the body itself: two lines drawn, one line true of every refusal Phase 1 can produce",
  },
  // ---- S7 · onboarding (#186). Every row is a clause or a node the flow cannot source, and each
  // shortens a box the frame draws taller or wider. Nothing here is a construction difference.
  {
    frame: "9a Folders",
    key: "div[1]/div[1]/div[0]/div[1]",
    props: ["box.h"],
    detail: "147 vs 107",
    issue: "#240",
    why: "the local card's `341 files / 2.1 GB` — nothing counts the files or bytes under a candidate folder, and the pair is not indexed yet because this is the screen that chooses it",
  },
  {
    frame: "9a Folders",
    key: "div[1]/div[1]/div[0]",
    props: ["box.h"],
    detail: "221.5 vs 181.5",
    issue: "#240",
    why: "the local side, one stats row shorter than the card it holds",
  },
  {
    frame: "9a Folders",
    key: "div[1]/div[1]/div[1]/div[1]",
    props: ["box.h"],
    detail: "147 vs 58",
    issue: "#99",
    why: "the remote card, missing both its stats row (#240) and `Browse Proton Drive…` — the daemon's `list` verb could feed a picker and nothing has built one",
  },
  {
    frame: "9a Folders",
    key: "div[1]/div[1]/div[1]",
    props: ["box.h"],
    detail: "221.5 vs 181.5",
    issue: "#241",
    why: "the remote side: the account line beneath the card has no source at all — the daemon reuses the CLI's session and never sees an address or a quota",
  },
  {
    frame: "9a Review",
    key: "div[1]/div[0]/div[0]/div[1]/span[1]",
    props: ["box.w"],
    detail: "71.47 vs 26.22",
    issue: "#191",
    why: "`files · 1.4 GB` going up, drawn as `files`: no level of the dry-run surface carries a size",
  },
  {
    frame: "9a Review",
    key: "div[1]/div[0]/div[1]/div[1]/span[1]",
    props: ["box.w"],
    detail: "82.63 vs 26.22",
    issue: "#191",
    why: "`files · 38.4 GB` coming down, the same clause on the other side",
  },
  {
    frame: "9a Review",
    key: "div[1]/div[1]",
    props: ["box.h"],
    detail: "163 vs 122",
    issue: "#242",
    why: "the fact strip without its first row — `11,798 files already match on both sides` counts files the plan does not act on, so `PlanSummary` has no field for it by construction",
  },
  {
    frame: "9a Review",
    key: "div[1]/div[2]/span[0]",
    props: ["box.w"],
    detail: "356.41 vs 165",
    issue: "#229",
    why: "`worked out 40 seconds ago · about 25 minutes to finish` minus the estimate — `run_dry_run` reports what would happen, never how long it would take. RE-PINNED from 158.41: the app was drawing its own `0 seconds ago` off the wall clock rather than the timing the fixture declares, so this width was whatever second the run landed on — see the note on `fixtures/clock.js`",
  },
  {
    frame: "9a Review",
    key: "div[1]/div[2]/span[1]",
    props: ["box.w"],
    detail: "467.58 vs 658.98",
    issue: "#229",
    why: "the same 198px, on the other side of the row: the spacer between the timing line and `See all 471 actions` takes exactly the width the missing estimate does not. The button itself (`div[1]/div[2]/button`) is drawn and asserted since #244",
  },
  {
    frame: "9a First sync",
    key: "div[0]/div[4]",
    props: ["box.w"],
    detail: "308.81 vs 136.02",
    issue: "#229",
    why: "`159 of 471 done · about 17 minutes left` minus the estimate; the fraction is `SyncActivity`'s own",
  },
  {
    frame: "9a Consent",
    key: "div[0]/div[1]",
    props: ["box.w"],
    detail: "538 vs 414.84",
    issue: "#207",
    why: "`12,480 files, 41.2 GB.` — no command reports index-wide totals, so the sub-line keeps its second sentence and drops its first",
  },
  {
    frame: "9a Consent",
    key: "div[0]/div[1]",
    props: ["box.h"],
    detail: "43.19 vs 21.59",
    issue: "#207",
    why: "the same line, one wrapped row instead of two. Pinned separately rather than as a wildcard: `box.w` is the only assertion sensitive to this node's TEXT, and it is the sentence that tells someone what happened to their files after the first merge",
  },
  {
    frame: "9a Consent",
    key: "div[0]",
    props: ["box.h"],
    detail: "178.19 vs 156.59",
    issue: "#207",
    why: "the head block, one wrapped line shorter for the same reason",
  },
  // MAINTAINER DECISION, 2026-08-17 (#218): the drawn command box (`sudo apt install proton-drive`)
  // never worked on any distribution, so it is dropped rather than made conditional — the frame
  // itself is unchanged and still draws it (DEVIATIONS §102). `cliMissingBody` grew a manual-path
  // sentence in its place, which is taller than the two sentences it replaced but still far short
  // of a command box plus its own Copy/Installation-help buttons, so the gap moved (60px → 40px)
  // rather than closing. All three rows are `decision`, not `issue`: #218 closes on this PR and
  // nothing further will shrink the gap.
  {
    frame: "9a CLI missing",
    key: "div/div",
    props: ["box.h"],
    detail: "176 vs 136",
    decision: true,
    why: "the text column without the command box, 40px taller than before the manual-path sentence — still short of the frame's command box and its buttons",
  },
  {
    frame: "9a CLI missing",
    key: "div",
    props: ["box.h"],
    detail: "176 vs 136",
    decision: true,
    why: "the row around it, the same 40px",
  },
  {
    frame: "9a CLI missing",
    key: "div/div/div[1]",
    props: ["box.h"],
    detail: "40 vs 60",
    decision: true,
    why: "the body paragraph itself: the manual-path sentence wraps to more lines than the two it replaced, which happened to fit the frame's 40px within tolerance. Not previously a separate row because it was not previously a mismatch",
  },

  // ---- S10 · the light theme. FIVE ROWS, AND EVERY ONE IS A DARK ROW SEEN TWICE.
  //
  // A light frame is its dark twin with the tokens swapped, so a capability the daemon does not have
  // is missing from both — `12a Settled light` draws the same `· 12,480 files · 41.2 GB` that
  // `2a Settled` does, and Phase 1 omits it in both themes. What it is NOT is the same row: a
  // deviation pins its measurement, and three of these five measure differently in light because
  // `--btn-primary-soft-border` does not exist there and the 2px a border takes out of a border-box
  // moves the card that holds the button. `4a Deletions`' card is 275.59 tall and its light twin
  // 273.59 — two pixels that would have been absorbed silently by a shared row.
  //
  // So they are written out rather than derived from the twin, and the two pixels are the argument.
  {
    frame: "12a Settled light",
    key: "div[0]/div[2]",
    props: ["box.w"],
    detail: "390.02 vs 195.02",
    issue: "#207",
    why: "the settled sub-line's `· 12,480 files · 41.2 GB` — G7 in light, the same missing index-wide totals as `2a Settled`, at the same 195px",
  },
  {
    frame: "12a Conflict light",
    key: "div[1]/div[1]/div[1]",
    props: ["box.w"],
    detail: "324.7 vs 145.31",
    issue: "#217",
    why: "`· last agreed 3 hours ago` needs a baseline timestamp `FileRecord` does not keep — `3a Conflict`'s deviation, drawn light",
  },
  {
    frame: "12a Deletions light",
    key: "div[1]/div[1]/div[0]/div[2]/div[1]/strong",
    props: ["box.w"],
    detail: "122.27 vs 104.56",
    decision: true,
    why: "the same word in the other theme, and the same 17.71px — `4a Deletions` records the decision",
  },
];

/** `frame|key|prop` → row, for the O(1) lookup the assertion loop wants. */
const INDEX = new Map(
  KNOWN_DEVIATIONS.flatMap((d) => d.props.map((prop) => [`${d.frame}|${d.key}|${prop}`, d])),
);

/**
 * Was this exact assertion expected to fail, WITH this exact difference?
 *
 * `detail` is what keeps a row from swallowing more than it names, and the hole it closes is not
 * hypothetical. `box.w` is the ONLY assertion on the settled sub-line that is sensitive to its text —
 * `width`/`height` are deliberately absent from `STYLE_PROPS` and no gate compares DOM text — so a
 * row that absorbed *any* `box.w` mismatch would leave that sentence entirely unchecked. Reword it,
 * drop a word from it, render the wrong timestamp: all silent.
 *
 * Pinning the measurement means the deviation absorbs the 195px Phase 1 actually draws and nothing
 * else. It has already earned it: a review agent editing this tree dropped `last ` from the sub-line,
 * which is a real divergence from `03-main-screen.md` and would have passed under the wildcard form.
 *
 * A row with no `detail` still matches on the property alone — deliberately, since a fit-gate row's
 * message carries a size nobody wants to re-type — but every style row should pin one.
 */
const hit = new Set();
export function isKnown(frame, key, prop, detail) {
  const found = INDEX.get(`${frame}|${key}|${prop}`);
  if (!found) return false;
  if (found.detail != null && found.detail !== detail) return false;
  hit.add(`${frame}|${key}|${prop}`);
  return true;
}

/**
 * The rows that did NOT fail — i.e. the deviations that have been closed and whose entries are now
 * lying about the state of the build. Returned rather than printed so the caller decides the
 * severity, which it does: this is a failure.
 */
export function unmetDeviations() {
  return KNOWN_DEVIATIONS.flatMap((d) =>
    d.props.filter((prop) => !hit.has(`${d.frame}|${d.key}|${prop}`)).map((prop) => ({ ...d, prop })),
  );
}

// ---- the blocks that render nothing --------------------------------------------------------
//
// THE SECOND LIST, AND IT EXISTS BECAUSE THE FIRST ONE CANNOT HOLD THESE. A deviation above is a
// node the app draws with a value the frame disagrees with — an assertion that fails, absorbed by
// name. This is the other shape: the frame draws a node and the app draws NOTHING there. No node,
// no assertion, nothing to absorb; a `KNOWN_DEVIATIONS` row for one of these would never fire, and
// the rule one screen up (an entry that stops failing fails the build) would reject it on sight.
//
// So the difference is invisible to the style gate by construction. `assert.mjs` compares STAMPED
// nodes, and an omitted block stamps nothing — S5's never-synced dialog rendered an empty body
// through four separate causes with every gate green. The report it grew afterwards listed the
// slots a fixture declares that the app never stamped, and that report is what this makes binding.
//
// TWO KINDS OF UNSTAMPED SLOT, AND ONLY ONE IS A FINDING:
//
//   1. The frame draws no node at that key either. `compactFids` is a factory over four tree shapes
//      and hands every frame the whole vocabulary, so `10a Settled` declaring `meta` means the
//      shape has a meta line, not that this panel does. `check-fixtures.mjs` tolerates exactly this
//      ("alive somewhere, not alive here") and argues the case at length. Inert, not wrong —
//      `assert.mjs` filters these out before it gets here. Eight of the twelve slots the first
//      version of this report listed were exactly this, which is why filtering at the source was the
//      fix rather than allow-listing the noise.
//   2. The frame draws the node and the app cannot. That is a Phase-1 omission like any other, and
//      it belongs here, with the issue that closes it.
//
// The staleness rule is the one above, transposed: a row that is no longer observed fails the
// build. Four things make that happen and every one wants a human: the app started stamping the
// slot (the capability landed, so delete the row), the prototype moved the node and the mapping
// followed it, the fixture stopped declaring the slot, or the frame left `frames/index.json`.
//
// A ROW IS ONE NODE — `frame`, `slot` AND `key` together. It reads like bookkeeping and it is the
// thing that keeps the list honest: since #248 a slot may be a FACTORY covering a run of siblings,
// so one `(frame, slot)` can be unstamped at four keys for four different reasons. `9a Review`'s
// `fact` row 0 is #242 and its row 3 would be something else entirely. Matching on `frame|slot`
// would let the first row's reason silently vouch for all of them.
//
// It is also what catches a moved node. That case is still observed — slot still drawn, still
// unstamped, merely at a different key — and identity that ignored the key would call it explained
// and stand behind a node nobody measured. Instead the row goes stale and the new key arrives
// unexplained, and both fail. Where a stale row and unexplained observations share a frame and a
// slot, the report lists them (`alsoUnstamped`) — candidates for where the node went, never a
// conclusion, because a run of siblings can gain a member in the same edit that moves one.
//
// The other three causes are indistinguishable from this data: each simply stops producing an
// observation. So the printed line names them rather than picking one. (The last is close to
// unreachable now — a frame carrying a `fids` map that stamps none of it fails as a blank frame in
// `assert.mjs`, so "no longer mapped" means the frame is gone, not gone quiet.)
//
// @property frame  the `data-screen-label` exactly as `frames/index.json` carries it
// @property slot   the fixture's fid slot name, as declared in `fixtures/fids.js`
// @property key    the ONE node key this row explains — for a factory slot, one row per key
// @property issue  the issue that closes it
// @property why    one line, in the same voice as DEVIATIONS.md
export const KNOWN_UNSTAMPED = [
  // `2a Syncing` draws a 460×2 track under the in-flight row with the fill at 64%; `2a Needs you`
  // draws the same track at 82%. One cause, four slots, and it is unreachable by construction
  // rather than unimplemented: `main.js` computes `bytes_done / bytes_total` and gets `null` on
  // every transfer the daemon can report. A bar at 0% would read as stalled and a bar at an
  // invented fraction would be worse, so `transferRow` draws no track at all.
  {
    frame: "2a Syncing",
    slot: "transferTrack",
    key: "div[1]/div[0]/div[0]/div[1]",
    issue: "#98",
    why: "`TransferActivity` carries `bytes_total` on an upload and `bytes_done` on a download and never both, so no percentage exists to draw (DEVIATIONS §63)",
  },
  {
    frame: "2a Syncing",
    slot: "transferFill",
    key: "div[1]/div[0]/div[0]/div[1]/div",
    issue: "#98",
    why: "the fill inside that track — same cause, and it goes with it",
  },
  {
    frame: "2a Needs you",
    slot: "transferTrack",
    key: "div[1]/div[0]/div/div[1]",
    issue: "#98",
    why: "the same track on the single-row column, drawn at 82% and unreachable for the same reason",
  },
  {
    frame: "2a Needs you",
    slot: "transferFill",
    key: "div[1]/div[0]/div/div[1]/div",
    issue: "#98",
    why: "the fill inside that track — same cause, and it goes with it",
  },

  // `4a Deletions` draws a two-fact strip under each card. `factsOf` builds one fact and only one:
  // `last edited <month year>` from the index's mtime. Everything else on the strip is a field the
  // index does not have, so the left card's strip is absent entirely and the right card's is missing
  // its first span — which is exactly why `cardFact` is keyed by the DRAWN slot and not by position.
  {
    frame: "4a Deletions",
    slot: "cardFact",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]/span[1]",
    issue: "#208",
    why: "`last opened Mar 2024` is an access time and `FileRecord` stores a modification time only",
  },

  // `5a Plan safe` puts a size beside every file it is about to move. The rehearsal's rows carry an
  // action and a path and nothing else, so `noteFor` answers null for a file row and `planSideRow`
  // draws no note — an em-dash there would be a number the plan does not have.
  {
    frame: "5a Plan safe",
    slot: "sideRowNote",
    key: "div[1]/div[1]/div[0]/div[2]/div[0]/span[2]",
    issue: "#191",
    why: "`1.2 MB` beside `docs/spec.md` — the dry-run report carries no per-file size (DEVIATIONS §76)",
  },
  {
    frame: "5a Plan safe",
    slot: "sideRowNote",
    key: "div[1]/div[1]/div[0]/div[2]/div[1]/span[2]",
    issue: "#191",
    why: "`2.8 MB` beside `photos/trip/img_0042.jpg` — same cause",
  },
  {
    frame: "5a Plan safe",
    slot: "sideRowNote",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]/span[2]",
    issue: "#191",
    why: "`96 KB` beside `notes/scratch.md` — same cause",
  },
  {
    frame: "5a Plan safe",
    slot: "sideRowNote",
    key: "div[1]/div[1]/div[1]/div[2]/div[0]/span[2]",
    issue: "#191",
    why: "`2.4 MB` beside `reports/q3-summary.pdf`, on the arriving side — same cause",
  },
  {
    frame: "5a Plan safe",
    slot: "sideRowNote",
    key: "div[1]/div[1]/div[1]/div[2]/div[1]/span[2]",
    issue: "#191",
    why: "`184 KB` beside `design/logo.svg` — same cause",
  },

  // `9a Folders`' remote side is thinner than its local one by exactly two nodes, and neither is a
  // drawing decision. Its third node — the path — IS drawn, as an `<input>`, and the fixture stops
  // declaring it rather than recording it here; see `cardPath` in fixtures/fids.js and §79.
  {
    frame: "9a Folders",
    slot: "cardButton",
    key: "div[1]/div[1]/div[1]/div[1]/button",
    issue: "#99",
    why: "`Browse Proton Drive…` has nowhere to go — nothing lists remote folders for a picker, which is why the remote root is typed and the local one is chosen (DEVIATIONS §79)",
  },
  {
    frame: "9a Folders",
    slot: "sideNote",
    key: "div[1]/div[1]/div[1]/div[2]",
    issue: "#241",
    why: "`Signed in as · 39.1 GB of 500 GB used` — no command reports the account or its quota, so the line is omitted rather than drawn with a blank name (DEVIATIONS §79)",
  },

  // `9a Review`'s first fact row. The other three are drawn; this one is keyed `at: 0` and never
  // built, which is what `factsBlock`'s `at` indirection exists for — the app's first row is the
  // frame's second, and comparing them by position would put a ringed dot against a filled one.
  {
    frame: "9a Review",
    slot: "fact",
    key: "div[1]/div[1]/div[0]",
    issue: "#242",
    why: "`11,798 files already match on both sides` — the dry-run summary counts what the plan will DO and nothing counts what it left alone (DEVIATIONS §79)",
  },
  {
    frame: "9a Review",
    slot: "factDot",
    key: "div[1]/div[1]/div[0]/span[0]",
    issue: "#242",
    why: "the dot on that row — it goes with it",
  },
  {
    frame: "9a Review",
    slot: "factLabel",
    key: "div[1]/div[1]/div[0]/span[1]",
    issue: "#242",
    why: "the label on that row — it goes with it",
  },
  {
    frame: "9a Review",
    slot: "factNote",
    key: "div[1]/div[1]/div[0]/span[2]",
    issue: "#242",
    why: "`left alone`, the note on that row — it goes with it",
  },

  // ---- S10 · the light theme. SIX ROWS, AND THEY ARE THEIR DARK TWINS' ROWS AT THE SAME KEYS.
  //
  // Unlike the deviations above, nothing here measures differently in light — an unstamped slot is
  // an absence, and an absence has no pixels to shift. They are written out for the reason the file
  // header gives: a row is one NODE, `frame` included, so a row on `4a Deletions` never vouches for
  // anything on `12a Deletions light`. Which is what caught them: the first version of this task
  // read the mapping off the raw registry entry instead of the resolved one, found `undefined` on
  // all seven light frames, iterated nothing, and reported six fewer blocks than the app omits —
  // the gate's own failure mode, on the frames it had just been pointed at.
  {
    frame: "12a Syncing light",
    slot: "transferTrack",
    key: "div[1]/div[0]/div[0]/div[1]",
    issue: "#98",
    why: "`2a Syncing`'s progress track, drawn light: `TransferActivity` carries `bytes_total` on an upload and `bytes_done` on a download and never both, so no percentage exists in either theme",
  },
  {
    frame: "12a Syncing light",
    slot: "transferFill",
    key: "div[1]/div[0]/div[0]/div[1]/div",
    issue: "#98",
    why: "the fill inside that track — same cause, and it goes with it",
  },
  {
    frame: "12a Deletions light",
    slot: "cardFact",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]/span[1]",
    issue: "#208",
    why: "`last opened Mar 2024` is an access time and `FileRecord` stores a modification time only",
  },
];

/**
 * Sort the run's drawn-but-unstamped slots against the list above.
 *
 * PURE, and taking the observations as an argument rather than accumulating them in module state
 * the way `isKnown` does. Two reasons: the classification is a set comparison that reads better in
 * one place than as a hit-set built a frame at a time, and a pure function is one a test can drive
 * without depending on what an earlier test happened to call.
 *
 * @param observed `{ frame, slot, key }` for every slot a frame DRAWS and the app did not stamp —
 *                 already filtered of the inert kind by the caller, and covering frames that
 *                 stamped NOTHING, which is the case the gate exists for
 * @returns recorded    — observations this list explains, for the report
 *          unexplained — observations it does not: a block that renders nothing, with no reason on
 *                        file. The finding this whole mechanism exists to make loud.
 *          stale       — rows whose exact node is no longer observed, each carrying `alsoUnstamped`
 *                        (every key of the same frame+slot that nothing explains) when there is one
 */
export function classifyUnstamped(observed) {
  const id = (r) => `${r.frame}|${r.slot}|${r.key}`;
  const rows = new Map(KNOWN_UNSTAMPED.map((r) => [id(r), r]));
  const recorded = [];
  const unexplained = [];
  const stale = [];

  for (const o of observed) {
    const row = rows.get(id(o));
    if (row) recorded.push({ ...o, issue: row.issue, why: row.why });
    else unexplained.push(o);
  }

  const seen = new Set(observed.map(id));
  for (const [key, row] of rows) {
    if (seen.has(key)) continue;
    // CANDIDATES, not a conclusion — and ALL of them, because a factory slot covers a run of
    // siblings and a run can gain a member in the same edit that moves one. Picking the first would
    // name an arbitrary one of several as "where the node went". Each is already in `unexplained`
    // and fails on its own; this only saves the reader from matching the two lists up by eye.
    const also = unexplained.filter((o) => o.frame === row.frame && o.slot === row.slot).map((o) => o.key);
    stale.push({ ...row, alsoUnstamped: also.length ? also : null });
  }
  return { recorded, unexplained, stale };
}
/**
 * Drawn nodes that **no fid slot claims** (#250), declared so their absence self-invalidates.
 *
 * # Why this list exists
 *
 * Every other suppression in this subsystem fails the build when it stops being true —
 * `KNOWN_DEVIATIONS` when a row stops mismatching, `KNOWN_UNSTAMPED` when a slot starts being
 * stamped, `SETTLED_FRAMES` when a label leaves `index.json`. **Leaving a drawn node undeclared had
 * no such rule**, and it removed that node from every gate at once: the style gate never compares
 * it, the unstamped gate never reports it, and `check-fixtures.mjs`'s "alive somewhere" rule keeps
 * a factory alive on index 0 without ever examining index 1.
 *
 * 270 nodes across 25 frames sat behind that absence. Each is declared below with a reason and a
 * class, and the classifier fails the build in **both** directions: an unclaimed node with no entry,
 * and an entry whose nodes are all claimed now.
 *
 * # The three classes, and what each one means
 *
 * * `decision` — the frame draws something this app deliberately does not, and there is nothing to
 *   build. A mock host desktop, a specimen sheet's own documentation table, a button whose absence
 *   is a recorded decision.
 * * `issue` — a capability gap with a number. The block would be built if the data existed.
 * * `mapping` — the app DRAWS the node and nobody declared a slot for it. These are the bugs this
 *   census exists to find, and they are recorded rather than fixed here on purpose: declaring a slot
 *   makes the node *compared*, which can surface unrelated failures, and #250's own instruction is
 *   that rushing the sort is how a plausible-looking reason ends up on file (§60).
 *
 * # Membership is pinned, deliberately
 *
 * Entries carry exact `keys`, never a prefix. A prefix rule would auto-excuse whatever a
 * re-extracted frame added underneath it — a suppression that widens itself, which is the shape
 * this list exists to remove.
 *
 * # The census that produced it
 *
 * #250 measured 268 unclaimed over the 36 frames that then carried a `fids` map. At the time of
 * writing all 51 do, and the true figure is **270 across 25 frames** (13.9% of 1,948 drawn). It is
 * not 280: `settingsShell.tab` is keyed by a tab **id**, `probeSlot` walks a numeric grid, and the
 * Settings pills therefore read as unclaimed while the app stamps them on every render. Counting a
 * stamped node as claimed removes those ten. See the loop in `assert.mjs`.
 */
export const KNOWN_UNCLAIMED = [
  {
    frame: "6a Activity passes",
    class: "issue",
    issue: "#229",
    why:
      "the twenty-bar duration chart and the per-pass row spans. The chart is drawn and NOT BUILT \u2014 " +
      "nothing in the app matches it, and `StatusHistoryEntry` records every attempt at a 20-entry " +
      "cap rather than the last twenty *passes*, so #250 names it a deliberate S6 call. The row " +
      "spans are a different thing wearing the same number: `sync_passes` landed with #229/#238 and " +
      "the app draws those rows, but the fixture declares each ROW and not the four spans inside " +
      "it, so they are a mapping gap rather than a capability one",
    keys: [
      "div[0]",
      "div[2]",
      "div[2]/div",
      "div[2]/div/div[0]",
      "div[2]/div/div[1]",
      "div[2]/div/div[1]/div[0]",
      "div[2]/div/div[1]/div[1]",
      "div[2]/div/div[1]/div[2]",
      "div[2]/div/div[1]/div[3]",
      "div[2]/div/div[1]/div[4]",
      "div[2]/div/div[1]/div[5]",
      "div[2]/div/div[1]/div[6]",
      "div[2]/div/div[1]/div[7]",
      "div[2]/div/div[1]/div[8]",
      "div[2]/div/div[1]/div[9]",
      "div[2]/div/div[1]/div[10]",
      "div[2]/div/div[1]/div[11]",
      "div[2]/div/div[1]/div[12]",
      "div[2]/div/div[1]/div[13]",
      "div[2]/div/div[1]/div[14]",
      "div[2]/div/div[1]/div[15]",
      "div[2]/div/div[1]/div[16]",
      "div[2]/div/div[1]/div[17]",
      "div[2]/div/div[1]/div[18]",
      "div[2]/div/div[1]/div[19]",
      "div[2]/div/div[2]",
      "div[2]/div/div[2]/span[0]",
      "div[2]/div/div[2]/span[1]",
      "div[3]/div[0]/span[0]",
      "div[3]/div[0]/span[1]",
      "div[3]/div[0]/span[2]",
      "div[3]/div[0]/span[3]",
      "div[3]/div[1]/span[0]",
      "div[3]/div[1]/span[1]",
      "div[3]/div[1]/span[2]",
      "div[3]/div[1]/span[3]",
      "div[3]/div[2]/div[0]",
      "div[3]/div[2]/div[0]/span[0]",
      "div[3]/div[2]/div[0]/span[1]",
      "div[3]/div[2]/div[0]/span[2]",
      "div[3]/div[2]/div[0]/span[3]",
      "div[3]/div[2]/div[1]",
      "div[3]/div[3]/span[0]",
      "div[3]/div[3]/span[1]",
      "div[3]/div[3]/span[2]",
      "div[3]/div[3]/span[3]",
      "div[3]/div[4]/span[0]",
      "div[3]/div[4]/span[1]",
      "div[3]/div[4]/span[2]",
      "div[3]/div[4]/span[3]",
      "div[3]/div[5]/span[0]",
      "div[3]/div[5]/span[1]",
      "div[3]/div[5]/span[2]",
      "div[3]/div[5]/span[3]",
      "div[3]/div[6]/span[1]",
    ],
  },
  {
    frame: "8a Schedule monthly",
    class: "decision",
    why:
      "the monthly schedule crop maps only its header, and cannot map more: this frame and `8a " +
      "Settings` disagree about the same panel's own numbers \u2014 the head row's gap (18px against " +
      "20px) and the sub-line's line-height (18.75px against 18.125px) \u2014 so a mapped node would " +
      "make the app fail whichever of the two it is not. The variant IS built and rendered (#193); " +
      "it is compared against the window frame instead. DEVIATIONS \u00a7104b",
    keys: [
      "div[0]",
      "div[0]/div[0]/div[1]",
      "div[0]/div[1]",
      "div[0]/div[1]/button[0]",
      "div[0]/div[1]/button[1]",
      "div[1]",
      "div[1]/div[0]",
      "div[1]/div[0]/span",
      "div[1]/div[0]/div",
      "div[1]/div[0]/div/button[0]",
      "div[1]/div[0]/div/button[1]",
      "div[1]/div[0]/div/button[2]",
      "div[1]/div[0]/div/button[3]",
      "div[1]/div[0]/div/button[4]",
      "div[1]/div[0]/div/button[5]",
      "div[1]/div[0]/div/button[6]",
      "div[1]/div[0]/div/button[7]",
      "div[1]/div[0]/div/button[8]",
      "div[1]/div[0]/div/button[9]",
      "div[1]/div[0]/div/button[10]",
      "div[1]/div[0]/div/button[11]",
      "div[1]/div[0]/div/button[12]",
      "div[1]/div[0]/div/button[13]",
      "div[1]/div[0]/div/button[14]",
      "div[1]/div[0]/div/button[15]",
      "div[1]/div[0]/div/button[16]",
      "div[1]/div[0]/div/button[17]",
      "div[1]/div[0]/div/button[18]",
      "div[1]/div[0]/div/button[19]",
      "div[1]/div[1]",
      "div[1]/div[1]/span",
      "div[1]/div[1]/div",
      "div[1]/div[1]/div/button[0]",
      "div[1]/div[1]/div/span",
      "div[1]/div[1]/div/button[1]",
      "div[1]/div[2]",
    ],
  },
  {
    frame: "10a Glyph states",
    class: "decision",
    why:
      "the specimen sheet's own documentation table \u2014 the column headers `mono` / `colour` / `what " +
      "it means`, and a prose description under each state. The app renders the glyph MARKS this " +
      "sheet is about (that is what the tray loads them from); the table explaining them is the " +
      "prototype's writing, and there is no screen it belongs to",
    keys: [
      "div[0]",
      "div[0]/div[0]",
      "div[0]/div[1]",
      "div[0]/div[2]",
      "div[0]/div[3]",
      "div[0]/div[4]",
      "div[0]/div[5]",
      "div[0]/div[6]",
      "div[0]/div[6]/div[0]",
      "div[0]/div[6]/div[1]",
      "div[0]/div[7]",
      "div[0]/div[8]",
      "div[0]/div[9]",
      "div[0]/div[10]",
      "div[0]/div[10]/div[0]",
      "div[0]/div[10]/div[1]",
      "div[0]/div[11]",
      "div[0]/div[12]",
      "div[0]/div[13]",
      "div[0]/div[14]",
      "div[0]/div[14]/div[0]",
      "div[0]/div[14]/div[1]",
      "div[0]/div[15]",
      "div[0]/div[16]",
      "div[0]/div[17]",
      "div[0]/div[18]",
      "div[0]/div[18]/div[0]",
      "div[0]/div[18]/div[1]",
      "div[0]/div[19]",
      "div[0]/div[20]",
      "div[0]/div[21]",
      "div[0]/div[22]",
      "div[0]/div[22]/div[0]",
      "div[0]/div[22]/div[1]",
      "div[1]",
    ],
  },
  {
    frame: "7a Activity quiet",
    class: "issue",
    issue: "#207",
    why:
      "the two side-count blocks (`12,480 files \u00b7 41.2 GB` on each side \u2014 G7, no command reports " +
      "index-wide totals), `next full check in 4m` (a countdown to a moment no reply carries, even " +
      "since #193 gave the daemon a schedule), and the `Last things to move` list beneath them",
    keys: [
      "div[0]",
      "div[2]/div[2]/div[0]/div[0]",
      "div[2]/div[2]/div[0]/div[1]",
      "div[2]/div[2]/div[0]/div[1]/span[0]",
      "div[2]/div[2]/div[0]/div[1]/span[1]",
      "div[2]/div[2]/div[0]/div[2]",
      "div[2]/div[2]/div[1]/div[0]",
      "div[2]/div[2]/div[1]/div[1]",
      "div[2]/div[2]/div[1]/div[1]/span[0]",
      "div[2]/div[2]/div[1]/div[1]/span[1]",
      "div[2]/div[2]/div[1]/div[2]",
      "div[3]/div[1]/div[0]",
      "div[3]/div[1]/div[0]/span[0]",
      "div[3]/div[1]/div[0]/span[1]",
      "div[3]/div[1]/div[0]/span[2]",
      "div[3]/div[1]/div[1]",
      "div[3]/div[1]/div[1]/span[0]",
      "div[3]/div[1]/div[1]/span[1]",
      "div[3]/div[1]/div[1]/span[2]",
      "div[3]/div[1]/div[1]/span[3]",
      "div[3]/div[1]/div[2]",
      "div[3]/div[1]/div[2]/span[0]",
      "div[3]/div[1]/div[2]/span[1]",
      "div[3]/div[1]/div[2]/span[2]",
      "div[3]/div[1]/div[2]/span[3]",
      "div[3]/div[1]/div[3]",
      "div[3]/div[1]/div[3]/span[0]",
      "div[3]/div[1]/div[3]/span[1]",
      "div[3]/div[1]/div[3]/span[2]",
      "div[3]/div[1]/div[3]/span[3]",
      "div[3]/div[1]/div[4]/span[1]",
      "div[3]/div[1]/div[4]/button[0]",
    ],
  },
  {
    frame: "7a File lookup",
    class: "issue",
    issue: "#190",
    why:
      "the four `This file's history` rows, which #250 names as a deliberate S5 omission. " +
      "`ControlCommand::Activity` has since landed the query behind them; the block itself is still " +
      "not built, so this stays recorded rather than becoming a mapping gap",
    keys: [
      "div[1]/div[2]/div[0]/div[1]/div[0]/span[1]",
      "div[1]/div[2]/div[1]/div[1]/div[0]/span[1]",
      "div[2]/div[0]",
      "div[2]/div[1]",
      "div[2]/div[1]/span[0]",
      "div[2]/div[1]/span[1]",
      "div[2]/div[1]/span[2]",
      "div[2]/div[2]",
      "div[2]/div[2]/span[0]",
      "div[2]/div[2]/span[1]",
      "div[2]/div[2]/span[2]",
      "div[2]/div[3]",
      "div[2]/div[3]/span[0]",
      "div[2]/div[3]/span[1]",
      "div[2]/div[3]/span[2]",
      "div[2]/div[4]",
      "div[2]/div[4]/span[0]",
      "div[2]/div[4]/span[1]",
      "div[2]/div[4]/span[2]",
      "div[2]/div[5]/span[0]",
    ],
  },
  {
    frame: "11a In situ",
    class: "decision",
    why:
      "the mock HOST DESKTOP this banner is drawn inside \u2014 a GNOME-style top bar (`Activities`, " +
      "`Tue 14:41`) and a tray strip of other applications' glyphs. The app cannot draw the desktop " +
      "it sits in, and the frame draws it to show the banner in place",
    keys: [
      "div[0]",
      "div[0]/span[0]",
      "div[0]/span[1]",
      "div[0]/span[2]",
      "div[0]/span[3]",
      "div[0]/span[3]/svg",
      "div[0]/span[3]/svg/path",
      "div[0]/span[3]/svg/circle",
      "div[0]/span[3]/span[0]",
      "div[0]/span[3]/span[1]",
      "div[0]/span[3]/span[2]",
      "div[0]/span[3]/span[3]",
      "div[1]",
      "div[1]/div",
      "div[1]/div/div[0]/div[0]/div/div[0]/span[0]",
      "div[1]/div/div[0]/div[0]/div/div[0]/span[1]",
    ],
  },
  {
    frame: "10a In situ",
    class: "decision",
    why: "the same mock host desktop as `11a In situ`, around the tray glyph rather than the banner",
    keys: [
      "div[0]",
      "div[0]/span[0]",
      "div[0]/span[1]",
      "div[0]/span[2]",
      "div[0]/span[3]",
      "div[0]/span[3]/span[0]",
      "div[0]/span[3]/span[0]/svg",
      "div[0]/span[3]/span[0]/svg/path",
      "div[0]/span[3]/span[0]/svg/circle",
      "div[0]/span[3]/span[1]",
      "div[0]/span[3]/span[2]",
      "div[0]/span[3]/span[3]",
      "div[0]/span[3]/span[4]",
    ],
  },
  {
    frame: "12a Tray light",
    class: "decision",
    why:
      "the same mock host desktop again, in light, plus the sheet's own caption about how the glyph " +
      "inverts",
    keys: [
      "div[0]",
      "div[0]/span[0]",
      "div[0]/span[1]",
      "div[0]/span[2]",
      "div[0]/span[3]",
      "div[0]/span[4]",
      "div[0]/span[5]",
      "div[1]",
    ],
  },
  {
    frame: "9a Folders",
    class: "issue",
    issue: "#241",
    why:
      "the two folder-count lines (`341 files \u00b7 2.1 GB`, `12,139 files \u00b7 39.1 GB` \u2014 G7's index-wide " +
      "totals again) and `you@proton.me`, which is G26: the daemon reuses the CLI's session for a " +
      "token and never sees an address",
    keys: [
      "div[1]/div[1]/div[0]/div[1]/div[1]",
      "div[1]/div[1]/div[0]/div[1]/div[1]/span[0]",
      "div[1]/div[1]/div[0]/div[1]/div[1]/span[1]",
      "div[1]/div[1]/div[1]/div[1]/div[0]",
      "div[1]/div[1]/div[1]/div[1]/div[1]",
      "div[1]/div[1]/div[1]/div[1]/div[1]/span[0]",
      "div[1]/div[1]/div[1]/div[1]/div[1]/span[1]",
      "div[1]/div[1]/div[1]/div[2]/span",
    ],
  },
  {
    frame: "12a Syncing light",
    class: "mapping",
    issue: "#377",
    why:
      "the seam's SECOND column's transfer row. The app renders one transfer list and stamps it " +
      "under the first column; the frame splits sent and received across the two seam columns, so " +
      "the incoming row is drawn at a key no slot names",
    keys: [
      "div[1]/div[1]/div",
      "div[1]/div[1]/div/div[0]",
      "div[1]/div[1]/div/div[0]/span[0]",
      "div[1]/div[1]/div/div[0]/span[1]",
      "div[1]/div[1]/div/div[0]/span[2]",
      "div[1]/div[1]/div/div[1]",
      "div[1]/div[1]/div/div[1]/div",
    ],
  },
  {
    frame: "2a Needs you",
    class: "mapping",
    issue: "#377",
    why: "the same second-column transfer row as `12a Syncing light`",
    keys: [
      "div[1]/div[1]/div",
      "div[1]/div[1]/div/div[0]",
      "div[1]/div[1]/div/div[0]/span[0]",
      "div[1]/div[1]/div/div[0]/span[1]",
      "div[1]/div[1]/div/div[0]/span[2]",
      "div[1]/div[1]/div/div[1]",
      "div[1]/div[1]/div/div[1]/div",
    ],
  },
  {
    frame: "2a Syncing",
    class: "mapping",
    issue: "#377",
    why: "the same second-column transfer row as `12a Syncing light`",
    keys: [
      "div[1]/div[1]/div",
      "div[1]/div[1]/div/div[0]",
      "div[1]/div[1]/div/div[0]/span[0]",
      "div[1]/div[1]/div/div[0]/span[1]",
      "div[1]/div[1]/div/div[0]/span[2]",
      "div[1]/div[1]/div/div[1]",
      "div[1]/div[1]/div/div[1]/div",
    ],
  },
  {
    frame: "9a CLI missing",
    class: "decision",
    why:
      "the copyable `sudo apt install proton-drive` box and the `Installation help` button. Both " +
      "are deliberately NOT built: the frozen frame draws a command this project cannot stand " +
      "behind for every distribution, and DEVIATIONS \u00a7102 records the decision to send people to " +
      "Proton's own instructions instead",
    keys: [
      "div/div/div[2]",
      "div/div/div[2]/span[0]",
      "div/div/div[2]/span[1]",
      "div/div/div[2]/button",
      "div/div/div[3]/button[1]",
    ],
  },
  {
    frame: "5a Checking",
    class: "decision",
    why:
      "the four footer doors, which #250 itself names as an S4 deliberate call \u2014 the checking state " +
      "draws them and the screen does not map them",
    keys: ["div[1]/div/span[0]", "div[1]/div/span[1]", "div[1]/div/span[2]", "div[1]/div/span[3]"],
  },
  {
    frame: "7a File pending",
    class: "mapping",
    issue: "#377",
    why: "a 3px progress bar and two flex spacers",
    keys: ["div[1]", "div[1]/div", "div[2]/span[1]"],
  },
  {
    frame: "12a Conflict light",
    class: "mapping",
    issue: "#377",
    why:
      "the conflict hexagon's inner `path` and its count `text`. `renderHexagon` draws both; the " +
      "fixture declares the `svg` and stops there",
    keys: ["div[1]/div[1]/svg/path", "div[1]/div[1]/svg/text"],
  },
  {
    frame: "3a Conflict",
    class: "mapping",
    issue: "#377",
    why: "the same hexagon internals as `12a Conflict light`",
    keys: ["div[1]/div[1]/svg/path", "div[1]/div[1]/svg/text"],
  },
  {
    frame: "3a Conflict diff",
    class: "mapping",
    issue: "#377",
    why: "the same hexagon internals, on the disclosure frame",
    keys: ["div[0]/svg/path", "div[0]/svg/text"],
  },
  {
    frame: "8a Settings",
    class: "mapping",
    issue: "#377",
    why: "the live panel's `event_driven_reconcile` key line and one flex spacer",
    keys: ["div[2]/div[4]/div[0]/div[2]", "div[2]/div[5]/div[1]/span[2]"],
  },
  {
    frame: "12a Settled light",
    class: "mapping",
    issue: "#377",
    why: "the header's flex spacer, which `settingsShell` declares and the main-screen map does not",
    keys: ["header/span[1]"],
  },
  {
    frame: "2a Settled",
    class: "mapping",
    issue: "#377",
    why: "the same header spacer as `12a Settled light`",
    keys: ["header/span[1]"],
  },
  {
    frame: "5a Plan",
    class: "decision",
    why: "`Leave it alone`, one of the two G3 buttons #250 names as an S4 deliberate call",
    keys: ["div[2]/div/button"],
  },
  {
    frame: "7a Never synced",
    class: "mapping",
    issue: "#377",
    why: "a flex spacer",
    keys: ["div[2]/span[1]"],
  },
  {
    frame: "8a Deletions tab",
    class: "mapping",
    issue: "#377",
    why:
      "the `deletion_policy \u00b7 applies to both directions` key line, which the app draws and this " +
      "crop does not declare",
    keys: ["div[3]"],
  },
  {
    frame: "8a Save refused",
    class: "issue",
    issue: "#236",
    why:
      "`Create it on Proton Drive`. No command creates a remote folder, and a live-looking button " +
      "that does nothing is the trap #224 and #227 already record \u2014 so S6 omits it",
    keys: ["div/div/div[3]/button[1]"],
  },
];

/**
 * Sort observed unclaimed nodes against [`KNOWN_UNCLAIMED`], both directions.
 *
 * Mirrors [`classifyUnstamped`] on purpose — same shape, same three buckets — because the two
 * answer opposite halves of one question and a reader who has understood one should not have to
 * learn a second vocabulary for the other.
 */
export function classifyUnclaimed(observed) {
  const id = (frame, key) => `${frame}|${key}`;
  const rows = new Map();
  for (const row of KNOWN_UNCLAIMED) {
    for (const key of row.keys) rows.set(id(row.frame, key), row);
  }
  const recorded = [];
  const unexplained = [];
  for (const o of observed) {
    const row = rows.get(id(o.frame, o.key));
    if (row) recorded.push({ ...o, class: row.class, issue: row.issue, why: row.why });
    else unexplained.push(o);
  }

  // The other direction: a declared node that is claimed now. Reported PER ENTRY rather than per
  // key, because an entry names a cluster and losing one of its members is the interesting event —
  // "these three of thirty-five" is what a reader needs to act on, and thirty-five separate lines
  // of which three matter is not.
  const seen = new Set(observed.map((o) => id(o.frame, o.key)));
  const stale = [];
  for (const row of KNOWN_UNCLAIMED) {
    const gone = row.keys.filter((key) => !seen.has(id(row.frame, key)));
    if (gone.length) stale.push({ ...row, gone });
  }
  return { recorded, unexplained, stale };
}
