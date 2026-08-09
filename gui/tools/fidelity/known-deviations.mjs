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
