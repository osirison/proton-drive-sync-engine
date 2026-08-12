// The assertions a screen CANNOT pass yet, each named with the issue that closes it.
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

/**
 * @property frame  the `data-screen-label` exactly as `frames/index.json` carries it
 * @property key    the node key, or `(fit)` for a fit-gate row
 * @property props  which assertions on that node are expected to fail — never a wildcard, so a
 *                  SECOND thing going wrong on the same node is still caught
 * @property detail the EXACT mismatch, verbatim as `assert.mjs` formats it. Absorbs that one
 *                  difference and nothing else; see the note below.
 * @property issue  the issue that closes it
 * @property why    one line, in the same voice as DEVIATIONS.md
 */
export const KNOWN_DEVIATIONS = [
  // ---- S9 · the three banners inside the desktop mock ----
  //
  // TWO SENTENCES ARE SHORTER THAN THE DRAWN ONES, and every row below is one of them wrapping to one
  // line instead of two — the banner, its head, the text column and the sentence itself, four nodes
  // per cause. Neither is a layout difference: `renderBanner` draws exactly what the frame draws, at
  // the length Phase 1 can honestly write.
  {
    frame: "11a In situ",
    key: "div[1]/div/div[0]",
    props: ["box.h"],
    detail: "170.5 vs 154.5",
    issue: "#208",
    why: "`1,204 photos` counts the files under a folder about to be deleted and nothing reports a subtree total (G8 #208); the app names the queue instead — one item, so the title is one line where the frame draws two",
  },
  {
    frame: "11a In situ",
    key: "div[1]/div/div[0]/div[0]",
    props: ["box.h"],
    detail: "93.5 vs 77.5",
    issue: "#208",
    why: "`1,204 photos` counts the files under a folder about to be deleted and nothing reports a subtree total (G8 #208); the app names the queue instead — one item, so the title is one line where the frame draws two",
  },
  {
    frame: "11a In situ",
    key: "div[1]/div/div[0]/div[0]/div",
    props: ["box.h"],
    detail: "93.5 vs 77.5",
    issue: "#208",
    why: "`1,204 photos` counts the files under a folder about to be deleted and nothing reports a subtree total (G8 #208); the app names the queue instead — one item, so the title is one line where the frame draws two",
  },
  {
    frame: "11a In situ",
    key: "div[1]/div/div[0]/div[0]/div/div[1]",
    props: ["box.h"],
    detail: "32 vs 16",
    issue: "#208",
    why: "`1,204 photos` counts the files under a folder about to be deleted and nothing reports a subtree total (G8 #208); the app names the queue instead — one item, so the title is one line where the frame draws two",
  },
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
    issue: "#187",
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. `panel.rs` applies them — against the click `Activate` reports, clamped into the work area, which is what makes it open upward on a bottom panel — so the design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["top"],
    detail: "40px vs auto",
    issue: "#187",
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. `panel.rs` applies them — against the click `Activate` reports, clamped into the work area, which is what makes it open upward on a bottom panel — so the design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["right"],
    detail: "16px vs auto",
    issue: "#187",
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. `panel.rs` applies them — against the click `Activate` reports, clamped into the work area, which is what makes it open upward on a bottom panel — so the design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["bottom"],
    detail: "38.5px vs auto",
    issue: "#187",
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. `panel.rs` applies them — against the click `Activate` reports, clamped into the work area, which is what makes it open upward on a bottom panel — so the design's intent is delivered by the thing that can hold it",
  },
  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["left"],
    detail: "662px vs auto",
    issue: "#187",
    why: "the drawn panel is absolutely positioned on a desktop MOCK; the shipped one is a borderless webview window whose viewport is the panel, so the offsets live on the window and not on any element. `panel.rs` applies them — against the click `Activate` reports, clamped into the work area, which is what makes it open upward on a bottom panel — so the design's intent is delivered by the thing that can hold it",
  },

  {
    frame: "10a In situ",
    key: "div[1]",
    props: ["border-top-color", "border-right-color", "border-bottom-color", "border-left-color"],
    detail: "rgba(255, 255, 255, 0.1) vs rgba(255, 107, 107, 0.3)",
    issue: "#187",
    why: "FOUR FRAMES AGAINST ONE, and DEVIATIONS §58d already ruled on it with only the four in view. `10-tray.md` asks the tray form for `border:1px solid rgba(255,255,255,.1)` because it floats over the desktop rather than over the app surface — and this is the one frame drawn that way, so it is the one that shows it. The four standalone `10a` panels all draw `#23262D` like every other compact panel, and they are gated. The app cannot be both. Keeping the four means the attention edge (`rgba(255,107,107,.3)`, measured on three needs-you frames) survives here, which is also the more useful of the two: it is the panel saying something is waiting on you. Recorded rather than resolved — the tie is the design's to break",
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
    issue: "#221",
    why: "the frame is a 522px WINDOW and the shell's is a fixed, non-resizable 1040 — S2 draws the body as a centred 520px column, which is the closest the shell can get, and a 520 column in a 1040 window has 260px either side where the frame's window has none",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[0]",
    props: ["margin-left"],
    detail: "0px vs 260px",
    issue: "#221",
    why: "the frame is a 522px WINDOW and the shell's is a fixed, non-resizable 1040 — S2 draws the body as a centred 520px column, which is the closest the shell can get, and a 520 column in a 1040 window has 260px either side where the frame's window has none",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]",
    props: ["padding-right"],
    detail: "24px vs 40px",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]",
    props: ["padding-left"],
    detail: "24px vs 40px",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div",
    props: ["gap"],
    detail: "22px vs 34px",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div",
    props: ["box.w"],
    detail: "472 vs 960",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div",
    props: ["box.h"],
    detail: "31 vs 32",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[0]",
    props: ["box.w"],
    detail: "43.86 vs 45.61",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[0]",
    props: ["box.h"],
    detail: "15 vs 16",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[1]",
    props: ["box.w"],
    detail: "63.59 vs 66.14",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[1]",
    props: ["box.h"],
    detail: "15 vs 16",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[2]",
    props: ["box.w"],
    detail: "48.19 vs 50.11",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[2]",
    props: ["box.h"],
    detail: "15 vs 16",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[3]",
    props: ["box.w"],
    detail: "39.63 vs 41.2",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },
  {
    frame: "3a Conflicts cleared",
    key: "div[1]/div/span[3]",
    props: ["box.h"],
    detail: "15 vs 16",
    issue: "#221",
    why: "the footer is a child of the WINDOW, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13. Faking those metrics at 1040 would draw four doors huddled inside 960px of chrome, which is neither frame",
  },

  // ---- S3 · the deletions screen. Four rows for one missing number and three for one window size.
  {
    frame: "4a Deletions",
    key: "div[1]/div[1]/div[0]/div[2]/div[1]",
    props: ["box.h"],
    detail: "41.59 vs 20.8",
    issue: "#208",
    why: "the folder card's consequence draws `1,204 photos, 8.4 GB` from a SUBTREE AGGREGATE, which no command produces and a directory's `file_size` is not. Phase 1 says `Deleting this folder removes everything inside it from this computer.` instead — true, and one line where the frame wraps to two",
  },
  {
    frame: "4a Deletions",
    key: "div[1]/div[1]/div[0]/div[2]/div[1]/strong",
    props: ["box.w"],
    detail: "122.27 vs 116.52",
    issue: "#208",
    why: "the emphasised loss is the aggregate itself. Phase 1 emphasises `everything inside it` — the same claim without a number, so the card keeps its crimson span and its structure rather than losing the emphasis with the figure",
  },
  {
    frame: "4a Deletions",
    key: "div[1]/div[1]/div[0]/div[2]",
    props: ["box.h"],
    detail: "275.59 vs 212.8",
    issue: "#208",
    why: "a card is as tall as what it says, and this one loses two things: the 20.79px second line of its consequence (#208), and its whole facts strip — a folder's two facts are the atime (#208) and the detected time, which is re-stamped every pass (#225), so there is no strip to draw",
  },
  {
    frame: "4a Armed",
    key: "div[0]/div[1]",
    props: ["box.h"],
    detail: "69.28 vs 46.19",
    issue: "#208",
    why: "the confirmation's sentence drops its `— 8.4 GB —` clause: the same aggregate, in the one place `05-deletions.md` makes it load-bearing. Three drawn lines become two. `DELETIONS.armedBody` takes a null size rather than an em-dash, which would be the app claiming the daemon answered `unknown` about how much is at stake",
  },
  {
    frame: "4a Empty",
    key: "div",
    props: ["margin-left"],
    detail: "0px vs 260px",
    issue: "#221",
    why: "the frame is a 522px WINDOW with no header and no doors, and the shell's is a fixed, non-resizable 1040 — S3 draws the body as a centred 520px column, which is the closest the shell can get, and a 520 column in a 1040 window has 260px either side where the frame's window has none",
  },
  {
    frame: "4a Empty",
    key: "div",
    props: ["margin-right"],
    detail: "0px vs 260px",
    issue: "#221",
    why: "the frame is a 522px WINDOW with no header and no doors, and the shell's is a fixed, non-resizable 1040 — S3 draws the body as a centred 520px column, which is the closest the shell can get, and a 520 column in a 1040 window has 260px either side where the frame's window has none",
  },
  {
    frame: "4a Empty",
    key: "div",
    props: ["box.h"],
    detail: "420 vs 662",
    issue: "#221",
    why: "the block is `flex: 1` between a 52px header and a 50px footer the frame does not draw, so it fills 662 of the window's 764 where the frame's whole surface is 422. Pinning it to 420 would leave the empty state floating in the top half of a window whose remaining space belongs to nothing",
  },

  // ---- S4 · the plan screen. Three causes: four rows for the byte totals nothing reports (#191),
  // four for two buttons that need a capability the daemon does not have (#192), and sixteen for a
  // 522px frame drawn inside a 1040px window (#221).
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
    issue: "#192",
    why: "the destructive band's body takes the width `Leave it alone` would have occupied — that button is the filtered apply reached from the band (G3), and `06-plan.md` says to hide a button rather than fake it",
  },
  {
    frame: "5a Plan",
    key: "div[2]/div/div/div[0]",
    props: ["box.w"],
    detail: "756.3 vs 884",
    issue: "#192",
    why: "the band's title, widened by the same absent button — the body is `flex: 1` and its children fill it",
  },
  {
    frame: "5a Plan",
    key: "div[2]/div/div/div[1]",
    props: ["box.w"],
    detail: "756.3 vs 884",
    issue: "#192",
    why: "the band's consequence sentence, widened by the same absent button; it still wraps to one line, so only the box moves",
  },
  {
    frame: "5a Plan",
    key: "div[4]/span",
    props: ["box.w"],
    detail: "207.52 vs 418.41",
    issue: "#192",
    why: 'the action bar\'s spacer absorbs the 199px `Run it without the deletion` (G3 #192), which `06-plan.md` is explicit about — "if unavailable, hide the button rather than faking it"',
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
    issue: "#221",
    why: "the frame is a 522px window against the shell's fixed, non-resizable 1040, so S4 draws the body as a centred 520px column — the closest it gets — and that column has 260px either side where the frame's window has none",
  },
  {
    frame: "5a Checking",
    key: "div[0]",
    props: ["margin-right"],
    detail: "0px vs 260px",
    issue: "#221",
    why: "the frame is a 522px window against the shell's fixed, non-resizable 1040, so S4 draws the body as a centred 520px column — the closest it gets — and that column has 260px either side where the frame's window has none",
  },
  {
    frame: "5a Checking",
    key: "div[0]/div[0]",
    props: ["box.h"],
    detail: "543 vs 542",
    issue: "#221",
    why: "the seam is pinned 60px off each end of its block, so it is exactly as tall as the block minus 120 — and the block is one pixel shorter here because the 1040 footer beneath it is one pixel taller than the 520 one (31px of doors against 32)",
  },
  {
    frame: "5a Checking",
    key: "div[1]",
    props: ["padding-right"],
    detail: "24px vs 40px",
    issue: "#221",
    why: "the footer is a child of the window, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13",
  },
  {
    frame: "5a Checking",
    key: "div[1]",
    props: ["padding-left"],
    detail: "24px vs 40px",
    issue: "#221",
    why: "the footer is a child of the window, not the body, so it stays 1040 wide while the frame's is 520 — the narrow window draws 24px padding, a 22px gap and 12.5px labels against the wide window's 32/34/13",
  },
  {
    frame: "5a Checking",
    key: "div[1]/div",
    props: ["gap"],
    detail: "22px vs 34px",
    issue: "#221",
    why: "the narrow window's doors sit 22px apart against the wide window's 34 — the same per-width footer metrics, on the same footer node",
  },
  {
    frame: "5a Checking",
    key: "div[1]/div",
    props: ["box.w"],
    detail: "472 vs 960",
    issue: "#221",
    why: "the door bar spans its window: 1040 less 32px either side here, 520 less 24 there",
  },
  {
    frame: "5a Checking",
    key: "div[1]/div",
    props: ["box.h"],
    detail: "31 vs 32",
    issue: "#221",
    why: "one pixel of line box, from the 13px labels the wide window draws against the narrow one's 12.5px",
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
    why: "`Proton Drive` loses the same count — and its sub-line too, but to a different gap: `next full check in 4m` counts down to a full-scan schedule the daemon does not expose (#193), and deriving it from `scan_interval` would contradict `6a Details`, which draws that interval as its own row",
  },
  {
    frame: "6a Activity passes",
    key: "div[3]/div[6]",
    props: ["box.h"],
    detail: "48 vs 35",
    issue: "#231",
    why: "the retention row keeps its sentence — `Only the last 20 passes are kept.` is true and needs no data — and loses `Open the system log`, which has no command behind it; the 31px button was what made the row 48 tall",
  },
  {
    frame: "7a File pending",
    key: "div[2]",
    props: ["box.h"],
    detail: "31 vs 14",
    issue: "#231",
    why: "the pending dialog's footer row keeps `only on this computer so far` and loses `Open folder`, the same missing opener; the row is now the height of its remaining text",
  },

  // ---- S6 · settings ----
  //
  // ELEVEN ROWS, FOUR CAUSES, AND EVERY ONE OF THEM IS A SENTENCE OR A CONTROL THAT IS NOT DRAWN.
  // Nothing here is a colour or a spacing that came out wrong: the tab bodies, the panels, the rule
  // rows and the three radio cards all match the frames exactly. What is left is the shape the
  // omissions leave behind — a helper one line shorter, a panel head that reaches the full width
  // because the schedule control is missing from beside it, and a tail that sits 71px lower because
  // the block above it was never built.
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
  {
    frame: "8a Settings",
    key: "div[2]/div[5]/div[0]/div[0]",
    props: ["box.w"],
    detail: "762.88 vs 938",
    issue: "#193",
    why: "the schedule panel's head row draws a Weekly/Monthly segmented control 155px wide beside its text, and G4 has no `full_scan_schedule` key for it to set — with the control gone the text block takes the width the control and its 20px gap were holding",
  },
  {
    frame: "8a Settings",
    key: "div[2]/div[5]/div[0]/div[0]/div[0]",
    props: ["box.w"],
    detail: "762.88 vs 938",
    issue: "#193",
    why: "the panel title inside that block, at the same width for the same reason",
  },
  {
    frame: "8a Settings",
    key: "div[2]/div[5]/div[0]/div[0]/div[1]",
    props: ["box.w"],
    detail: "762.88 vs 938",
    issue: "#193",
    why: "and its sub-line. The height is unaffected: both sentences are one line at either width",
  },
  {
    frame: "8a Skip rules",
    key: "div[2]/div[2]",
    props: ["margin-top"],
    detail: "85.8125px vs 156.812px",
    issue: "#232",
    why: "the tail is pushed to the bottom with `margin-top:auto`, so its used value measures whatever is above it — and what is above it is 71px shorter without the unsyncable panel (G19: a socket or a symlink never enters the index, so there is nothing to count and `See them` would open the one group `7a Never synced` already omits)",
  },
  {
    frame: "8a Skip rules",
    key: "div[2]/div[2]/div[1]",
    props: ["margin-top"],
    detail: "12px vs 0px",
    issue: "#232",
    why: "the `.sync` note's 12px separates it from that panel, so with the panel gone the margin belongs to nothing — setting it anyway would be spacing a block against something that is not there",
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
    why: "the remote card, missing both its stats row (#240) and `Browse Proton Drive…` — `list_remote` reads a path and there is no picker for one",
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
    detail: "356.41 vs 158.41",
    issue: "#229",
    why: "`worked out 40 seconds ago · about 25 minutes to finish` minus the estimate — `run_dry_run` reports what would happen, never how long it would take",
  },
  {
    frame: "9a Review",
    key: "div[1]/div[2]",
    props: ["box.h"],
    detail: "31 vs 14",
    issue: "#244",
    why: "the same row without `See all 471 actions`: the action list is the Plan screen, behind a door the takeover covers",
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
  {
    frame: "9a CLI missing",
    key: "div/div",
    props: ["box.h"],
    detail: "176 vs 116",
    issue: "#218",
    why: "the text column without the command box: every command in `CLI_INSTALL_COMMANDS` names a package that is in no distribution's repository, so there is nothing true to put in it",
  },
  {
    frame: "9a CLI missing",
    key: "div",
    props: ["box.h"],
    detail: "176 vs 116",
    issue: "#218",
    why: "the row around it, the same 60px",
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
    key: "div[1]/div[1]/div[0]/div[2]",
    props: ["box.h"],
    detail: "273.59 vs 210.8",
    issue: "#208",
    why: "the folder card without its second consequence line and without its facts strip — `4a Deletions`' 275.59/212.8, each two pixels shorter because light's primary-soft button draws no border and the card is as tall as what it holds",
  },
  {
    frame: "12a Deletions light",
    key: "div[1]/div[1]/div[0]/div[2]/div[1]",
    props: ["box.h"],
    detail: "41.59 vs 20.8",
    issue: "#208",
    why: "the consequence itself, two drawn lines against Phase 1's one — the subtree aggregate `1,204 photos, 8.4 GB` has no command behind it in either theme",
  },
  {
    frame: "12a Deletions light",
    key: "div[1]/div[1]/div[0]/div[2]/div[1]/strong",
    props: ["box.w"],
    detail: "122.27 vs 116.52",
    issue: "#208",
    why: "the emphasised loss, `everything inside it` where the frame draws the aggregate — the same substitution `4a Deletions` records, and the same 5.75px",
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
    slot: "cardFacts",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]",
    issue: "#208",
    why: "the folder card's strip, absent rather than partial: its two facts are `last opened Mar 2024` (an atime — the index stores mtime only) and `deleted on Proton 22m ago` (#225), so `factsOf` returns no facts for a directory and the strip is never built",
  },
  {
    frame: "4a Deletions",
    slot: "cardFact",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]/span[0]",
    issue: "#225",
    why: "`deleted on Proton 22m ago` — `detected_epoch_secs` is re-stamped on every pass, so it is the age of the pass and not of the deletion (DEVIATIONS §75)",
  },
  {
    frame: "4a Deletions",
    slot: "cardFact",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]/span[1]",
    issue: "#208",
    why: "`last opened Mar 2024` is an access time and `FileRecord` stores a modification time only",
  },
  {
    frame: "4a Deletions",
    slot: "cardFact",
    key: "div[1]/div[1]/div[1]/div[2]/div[2]/span[0]",
    issue: "#225",
    why: "`deleted here 6m ago` on the local card — the same re-stamped field as the Proton one, which is why this strip draws its second fact and not its first",
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
    slot: "cardFacts",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]",
    issue: "#208",
    why: "the folder card's facts strip, absent rather than partial — an atime the index does not store (#208) and a detected time that is the pass's age (#225), so `factsOf` returns nothing for a directory",
  },
  {
    frame: "12a Deletions light",
    slot: "cardFact",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]/span[0]",
    issue: "#225",
    why: "`deleted on Proton 22m ago` — `detected_epoch_secs` is re-stamped every pass",
  },
  {
    frame: "12a Deletions light",
    slot: "cardFact",
    key: "div[1]/div[1]/div[0]/div[2]/div[2]/span[1]",
    issue: "#208",
    why: "`last opened Mar 2024` is an access time and `FileRecord` stores a modification time only",
  },
  {
    frame: "12a Deletions light",
    slot: "cardFact",
    key: "div[1]/div[1]/div[1]/div[2]/div[2]/span[0]",
    issue: "#225",
    why: "`deleted here 6m ago` on the local card — the same re-stamped field, which is why this strip draws its second fact and not its first",
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
