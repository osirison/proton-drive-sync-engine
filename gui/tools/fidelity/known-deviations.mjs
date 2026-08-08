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
 * @property issue  the issue that closes it
 * @property why    one line, in the same voice as DEVIATIONS.md
 */
export const KNOWN_DEVIATIONS = [
  {
    frame: "2a Settled",
    key: "div[0]/div[2]",
    props: ["box.w"],
    issue: "#207",
    why: "the settled sub-line's `· 12,480 files · 41.2 GB` is G7 — no command reports index-wide totals, so Phase 1 draws `last synced 2 minutes ago` alone and the line measures 195px against the drawn 390px",
  },
];

/** `frame|key|prop` → row, for the O(1) lookup the assertion loop wants. */
const INDEX = new Map(
  KNOWN_DEVIATIONS.flatMap((d) => d.props.map((prop) => [`${d.frame}|${d.key}|${prop}`, d])),
);

/** Was this exact assertion expected to fail? Records the hit so `unmet()` can report the rest. */
const hit = new Set();
export function isKnown(frame, key, prop) {
  const found = INDEX.get(`${frame}|${key}|${prop}`);
  if (found) hit.add(`${frame}|${key}|${prop}`);
  return Boolean(found);
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
