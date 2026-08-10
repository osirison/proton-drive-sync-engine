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
    why: "the tail is pushed to the bottom with `margin-top:auto`, so its used value measures whatever is above it — and what is above it is 71px shorter without the unsyncable panel (G15: a socket or a symlink never enters the index, so there is nothing to count and `See them` would open the one group `7a Never synced` already omits)",
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
    why: "the refusal's body drops `Create the folder on Proton Drive first, or pick a different one.` — `write_config` validates TOML and never contacts Proton Drive, so it cannot know a remote folder is missing (G16) — and one line instead of two takes 20px off the dialog",
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
    frame: "9a First sync",
    key: "div[1]/span[1]",
    props: ["box.w"],
    detail: "174.55 vs 438.25",
    issue: "#213",
    why: "the footer spacer, grown by the sentence beside it: `nothing deleted · 2 conflicts kept as copies` is a claim about the pass in flight, and no reply carries a per-pass summary while it runs",
  },
  {
    frame: "9a Consent",
    key: "div[0]/div[1]",
    props: ["box.w", "box.h"],
    detail: null,
    issue: "#207",
    why: "`12,480 files, 41.2 GB.` — no command reports index-wide totals, so the sub-line keeps its second sentence and drops its first, and wraps to one line instead of two",
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
