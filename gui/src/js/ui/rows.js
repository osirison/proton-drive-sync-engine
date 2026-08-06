// Rows (F5). The list primitives the screens compose: the transfer row on the main screen and in
// the compact panel, the four flat rows that carry a fact, a sync pass, a history entry or an
// action, and the deletion card.
//
// MEASURED out of the 51 fixtures F8 extracted, then read back against the prototype for the values
// a fixture cannot carry — `box` records width and height but no position, so the vertical rhythm
// inside a card is not derivable from JSON at all. Three things shape the API:
//
//   · THE FLAT ROWS ARE ONE SHAPE AT SEVERAL RUNGS. history, path, fact, pass and action are all
//     `padding:<y> 2px` over a 1px `--divider` rule, flex, centred, with a gap. Only y and the gap
//     move: 9/13, 9/12, 11/14, 12/14, 13/14. Modelling them as five unrelated builders would hide
//     that and let the sixth screen invent a sixth rhythm.
//     THE FIVE HERE ARE NOT THE WHOLE SET. `5a Plan` draws nine rows at `8px 2px` gap-13 with the
//     rule on the BOTTOM rather than the top — eleven border-bottom rows across the prototype
//     against ninety-two border-top. That is S4's screen and not F5's scope, so it is not modelled
//     here; the ladder is open, and a screen that needs a rung adds one rather than assuming these
//     five are exhaustive. DEVIATIONS §53a.
//   · MIRRORING IS NOT ONE RULE. Two things straddle the seam and they flip in OPPOSITE
//     directions — see `transferRow` and `deletionCard`. Generalising either one produces a wrong
//     screen, which is why they are written out separately rather than sharing a `side` helper.
//   · THE DOT AND THE EYEBROW BELONG TO NOBODY. Both appear in rows, in bands and inside dialogs.
//     They live here because a row is where they are load-bearing; bands.js imports them.
//
// Every filename, size, timestamp and path is IBM Plex Mono — that is the design's rule and it is
// nearly universal here. The one deliberate exception is the deletion card's headline, which is
// 16px sans: it is naming a *thing you are about to lose*, not printing a path.

import { el } from "./el.js";
import { button } from "./controls.js";

// ------------------------------------------------------------------------------ primitives ----

/**
 * The severity dot. Three tones and three sizes, all drawn.
 *
 * `decision` is an OUTLINE and the other two are fills — that is the whole grammar of the design's
 * crimson: an outline is a choice waiting for you, a fill is a thing that will happen. A solid
 * `decision` dot would say the conflict is already resolved.
 *
 * Ring width defaults off size: 2px at 7 and 8, 1px at 6. That holds for everything drawn — the
 * prototype writes `border:1.5px` on `5a Plan`'s conflict dot, but Chromium floors a sub-pixel
 * border at DSF 1, so the frame records 1px and there is not a single 1.5px border in any of the 51
 * fixtures. Do not "restore" the 1.5px: it renders identically and only makes the source disagree
 * with the ground truth. The override is here for a size nothing has drawn yet, not for that.
 *
 * A 2px ring on a 6px dot leaves a 2px hole and reads as a fill. That is the one hard constraint.
 */
export function dot({ tone = "inert", size = 7, ring = null } = {}) {
  const node = el("span", { class: `dot dot-${tone}` });
  node.style.width = `${size}px`;
  node.style.height = `${size}px`;
  if (tone === "decision") node.style.borderWidth = ring ?? (size <= 6 ? "1px" : "2px");
  return node;
}

/**
 * The 10px uppercase section label above a group. `.16em` tracking at 10px is the 1.6px the frames
 * measure; it is written as an em so it survives a font-size override.
 *
 * `up`/`down` are the directional labels (`This computer` / `Proton Drive`), `neutral` the grey one
 * over a plain group, `decision` the crimson one over a deletion. `align: "end"` for anything in
 * the right-hand column — see the note on mirroring in `deletionCard`.
 */
export function eyebrow({ tone = "neutral", text, align = "start" } = {}) {
  return el("div", { class: `eyebrow eyebrow-${tone} eyebrow-${align}` }, text);
}

// --------------------------------------------------------------------------- transfer rows ----

/**
 * A file moving between the two sides. `2a Syncing` draws four of them, `2a Compact syncing` two.
 *
 * THE ARROW SITS BESIDE THE SEAM, and both `03-main-screen.md` and issue #169 say the opposite:
 * *"the arrow is on the outside edge in both columns, pointing away from the seam"*. All four
 * frames that draw this — `2a Syncing`, `2a Compact syncing` and both light twins — put the `→`
 * LAST in the leaving column and the `←` FIRST in the arriving one, so both arrows land next to the
 * centre line and both point across it. The prose contradicts its own preceding sentence, which
 * gives the element order correctly. Frames win under the §1.3 precedence rule. DEVIATIONS §53.
 *
 * It is a rotation rather than a mirror: `[name][size][arrow]` becomes `[arrow][name][size]`, so the
 * name still reads before the size on both sides. A true mirror would put the size first on the
 * right, and nothing draws that.
 *
 * Placement follows DIRECTION, not column. The compact panel is a single 360px column with no seam
 * and no second side, and its arriving row still leads with the arrow.
 */
export function transferRow({
  direction = "up",
  name,
  detail = null,
  state = "active",
  progress = 0,
  size = "standard",
} = {}) {
  const compact = size === "compact";
  const slots = {
    arrow: el("span", { class: "transfer-arrow" }, direction === "up" ? "→" : "←"),
    name: el("span", { class: "transfer-name" }, name),
    // The compact panel drops the size chip entirely — 332px of row has no room for it, and the
    // filename is the thing you are looking for.
    detail: detail && !compact ? el("span", { class: "transfer-detail" }, detail) : null,
  };
  const parts = transferSlotOrder(direction).map((slot) => slots[slot]);

  const row = el(
    "div",
    { class: `transfer-row transfer-${state} transfer-${direction}` + (compact ? " is-compact" : "") },
    parts.filter(Boolean),
    // A queued row has no bar at all rather than a bar at 0% — an empty track reads as stalled.
    state === "active" ? progressBar(progress) : null,
  );
  return row;
}

/**
 * Which order the three slots go in, given the direction. Pure, exported and tested on its own,
 * because this is the one fact in the module that BOTH the design doc and the issue state
 * backwards — the next person to read either will "fix" it, and the app will still look plausible.
 * A test is the only place that argument is settled in advance.
 */
export function transferSlotOrder(direction) {
  if (direction === "up") return ["name", "detail", "arrow"];
  if (direction === "down") return ["arrow", "name", "detail"];
  throw new Error(`rows: transfer direction must be "up" or "down", got "${direction}"`);
}

function progressBar(progress) {
  const fill = el("div", { class: "transfer-fill" });
  fill.style.width = `${Math.max(0, Math.min(1, progress)) * 100}%`;
  return el("div", { class: "transfer-track" }, fill);
}

// -------------------------------------------------------------------------- the flat rows ----

/**
 * The shared body of the four flat rows. Not exported: a screen wanting a fifth rhythm should
 * measure it and add a rung here, the same discipline controls.js applies to button sizes.
 */
function flatRow(kind, children, { class: cls = null, ...rest } = {}) {
  return el("div", { class: ["row", `row-${kind}`, cls].filter(Boolean).join(" "), ...rest }, children);
}

/**
 * A statement of fact with a dot beside it, in prose. `9a Review`'s merge summary — the four rows
 * that say what the first sync will and will not touch. The note on the right is the qualifier:
 * `left alone`, `nothing is deleted`.
 *
 * NOT `7a Never synced`'s entries, which look similar and are not this: no dot, `gap:12`, and a
 * mono subject rather than a sans sentence. That is `pathRow` below. Two rows this close together
 * are exactly where a shared builder gets stretched until it fits neither.
 */
export function factRow({ tone = "inert", label, note = null } = {}) {
  return flatRow("fact", [
    dot({ tone, size: 6 }),
    el("span", { class: "row-label" }, label),
    note ? el("span", { class: "row-note" }, note) : null,
  ]);
}

/**
 * A file named by its path, with a qualifier. `7a Never synced` draws four.
 *
 * The subject is mono because it is a path, and there is no dot — nothing here has a severity, which
 * is the screen's whole point: *nothing here is at risk, it's just not backed up*.
 *
 * `dim` drops the path from `--text-2` to `--text-3`, which is how the frame separates the two
 * groups: files you chose to skip read at full strength, files that cannot be synced at all read
 * quieter. `mono` on the note follows the design's own division — `2.1 MB` is data and sets in
 * mono, `a socket` is prose and does not.
 */
export function pathRow({ path, note = null, mono = false, dim = false } = {}) {
  return flatRow("path", [
    el("span", { class: "path-name" + (dim ? " is-dim" : "") }, path),
    note ? el("span", { class: "path-note" + (mono ? " is-mono" : "") }, note) : null,
  ]);
}

/**
 * One sync pass. `6a Activity passes` draws six, one of them failed.
 *
 * A FAILED PASS CARRIES THE DAEMON'S EXACT STRING and never a paraphrase of it — that rule is in
 * S5's definition of done, and it is the difference between a user who can search for their problem
 * and one who cannot. The error block is `--surface` inside a tinted row on purpose: it is a quote,
 * so it is set apart from the sentence quoting it.
 *
 * The failed row also BREAKS OUT of the list's inset — `margin:0 -12px` against `padding:12px`,
 * against the plain row's `padding:12px 2px` — so the tint runs wider than the rows above it. It is
 * rounded at the bottom only, because it hangs below the row it belongs to.
 */
export function passRow({ outcome = "clean", label, detail = null, time = null, error = null } = {}) {
  const failed = outcome === "failed";
  const head = [
    dot({ tone: failed ? "decision" : "inert", size: 7 }),
    el("span", { class: "row-label" }, label),
    detail ? el("span", { class: "pass-detail" }, detail) : null,
    time ? el("span", { class: "pass-time" }, time) : null,
  ].filter(Boolean);

  if (!failed) return flatRow("pass", head);

  return el(
    "div",
    { class: "row row-pass is-failed" },
    el("div", { class: "pass-head" }, head),
    error ? el("div", { class: "pass-error" }, error) : null,
  );
}

/**
 * One entry in a file's past. `7a File lookup`'s history block.
 *
 * Leads with the direction arrow, or with an outlined dot when the entry is a decision someone made
 * rather than a transfer that happened (`Both sides had changed — you kept yours`).
 *
 * `emphasis` brightens the label from `--text-2` to `--text-bright`. Exactly one row is drawn that
 * way and it is both the first and the only one from today, so the frame cannot say which of those
 * it means. Modelled as "most recent" because that is the one the caller can always answer.
 *
 * Phase 1 has no consumer: the history block needs a per-path history query the daemon does not
 * have, so S5 omits it until G1 (IMPLEMENTATION-PLAN §4 row 7). Built anyway — it is drawn, it is
 * measured, and the gate can check it the day the query lands.
 */
export function historyRow({ direction = null, label, time = null, emphasis = false } = {}) {
  const lead = direction
    ? el("span", { class: `history-arrow history-${direction}` }, direction === "up" ? "→" : "←")
    : dot({ tone: "decision", size: 6 });
  return flatRow("history", [
    lead,
    el("span", { class: "row-label" + (emphasis ? " is-bright" : "") }, label),
    time ? el("span", { class: "row-time" }, time) : null,
  ]);
}

/**
 * A row that ends in a button. `8a Skip rules` draws three, and they are the only ones drawn —
 * `7a Never synced`'s `Change this rule` looks like it belongs here and does not: it is a
 * standalone `inline-block` button sitting after the group, at controls.js's plain `small` size.
 *
 * `lead` is the subject in mono at a fixed 180px — a skip rule is a pattern, and patterns line up
 * or they cannot be compared. `action` takes a built control rather than a label so the caller
 * chooses the kind; every one drawn so far is a `secondary`.
 */
export function actionRow({ lead = null, title, note = null, action = null } = {}) {
  return flatRow("action", [
    lead ? el("span", { class: "action-lead" }, lead) : null,
    el(
      "div",
      { class: "action-body" },
      el("div", { class: "action-title" }, title),
      note ? el("div", { class: "action-note" }, note) : null,
    ),
    action,
  ]);
}

// -------------------------------------------------------------------------- deletion cards ----

/**
 * A file waiting to be deleted. `4a Deletions` draws one of each severity, `4a Compact` both again
 * at panel scale.
 *
 * THE EYEBROW MIRRORS THE OTHER WAY FROM THE TRANSFER ROW. Here the dot sits on the OUTSIDE edge —
 * `[dot][PERMANENT · THIS COMPUTER]` on the left, `[RECOVERABLE · PROTON DRIVE][dot]` on the right,
 * with the whole column's text right-aligned. The transfer row keeps its arrow beside the seam.
 * Two rules, two directions, one screen apart; there is no shared `side` helper on purpose,
 * because the next person to write one will make them agree and one of them will be wrong.
 *
 * SEVERITY SORTS ACROSS THE SEAM: permanent left, recoverable right (S3). The band tint follows
 * severity, not side, and neither band is ever a solid fill — the only solid red in the app is the
 * armed `Delete permanently` button.
 *
 * `emphasis` is the substring of `consequence` to set in 600. It is COLOURED only when the deletion
 * is permanent: `1,204 photos, 8.4 GB` goes crimson because it is what you lose, while the
 * recoverable card's `Proton Drive's Trash` is bolded and left the body colour because it is where
 * the file goes, not a loss. Passing the substring rather than pre-split nodes keeps the sentence
 * whole in `copy.js`, which is what the copy gate asserts against.
 *
 * NO COMPACT VARIANT HERE. `4a Compact` draws both severities at 332x61 — a fifth of this card's
 * height, with no facts strip, no gate and no second button. That is not this card at a smaller
 * size, it is a different component, and it belongs to F6 with the rest of the 360px panel. It also
 * needs two band alphas that no token carries yet (DEVIATIONS §52a). A `compact` flag here would
 * have to either lie or grow a second layout, so there isn't one.
 */
export function deletionCard({
  severity = "permanent",
  eyebrowText,
  policy = null,
  name,
  kind = null,
  consequence,
  emphasis = null,
  facts = [],
  gate = null,
  action = null,
  keep = null,
} = {}) {
  if (severity !== "permanent" && severity !== "recoverable")
    throw new Error(`rows: deletionCard severity must be "permanent" or "recoverable", got "${severity}"`);

  const side = severity === "permanent" ? "start" : "end";
  const severityDot = dot({ tone: severity === "permanent" ? "destructive" : "decision", size: 8 });
  // The shared `.eyebrow` typography, not a second copy of it — the alignment the standalone
  // builder applies is the flex row's job here, so this takes the class without going through
  // `eyebrow()`. Wearing only a local class is how the first version of this rendered at 16px sans.
  const label = el("span", { class: "eyebrow eyebrow-decision" }, eyebrowText);

  return el(
    "div",
    { class: `deletion deletion-${severity} deletion-${side}` },
    el("div", { class: "deletion-head" }, side === "start" ? [severityDot, label] : [label, severityDot]),
    policy ? el("div", { class: "deletion-policy" }, policy) : null,
    el(
      "div",
      { class: "deletion-card" },
      el(
        "div",
        { class: "deletion-title" },
        el("span", { class: "deletion-name" }, name),
        kind ? el("span", { class: "deletion-kind" }, kind) : null,
      ),
      el("div", { class: "deletion-consequence" }, ...emphasise(consequence, emphasis)),
      facts.length
        ? el(
            "div",
            { class: "deletion-facts" },
            facts.map((fact) => el("span", { class: "deletion-fact" }, fact)),
          )
        : null,
      gate,
      action ? el("div", { class: "deletion-action" }, action) : null,
      keep,
    ),
  );
}

/**
 * Split a sentence into `[before, match, after]`, or `[sentence]` when there is nothing to
 * emphasise or the substring is not in it.
 *
 * FALLING BACK TO THE WHOLE SENTENCE IS THE BEHAVIOUR THAT MATTERS, and it is why this is pure and
 * tested. The copy deck stores these sentences whole and the emphasis is a substring of one — so an
 * edit to `copy.js` can silently stop matching. Returning the sentence intact means the screen
 * loses a bold span; the tempting alternatives lose the sentence, and the copy gate would then fail
 * somewhere that looks nothing like the cause.
 */
export function splitEmphasis(sentence, substring) {
  if (!substring) return [sentence];
  const at = sentence.indexOf(substring);
  if (at < 0) return [sentence];
  return [sentence.slice(0, at), substring, sentence.slice(at + substring.length)];
}

/** `splitEmphasis` as nodes, for `el`'s children. */
function emphasise(sentence, substring) {
  const parts = splitEmphasis(sentence, substring);
  if (parts.length === 1) return parts;
  const [before, match, after] = parts;
  return [before, el("strong", { class: "deletion-emphasis" }, match), after];
}

/**
 * The armed gate's hint line: `To delete it, type DELETE below.` with the word set in mono crimson.
 *
 * The word is spelled in caps everywhere the copy deck uses it, and this is the one place voice
 * rule 5 (no shouting) is deliberately broken — `delete` is a word people type by habit, `DELETE`
 * is not. Pairs with controls.js's `deleteGate`, which enforces the case.
 */
export function deleteHint(sentence, word = "DELETE") {
  const at = sentence.indexOf(word);
  if (at < 0) return el("div", { class: "deletion-hint" }, sentence);
  return el(
    "div",
    { class: "deletion-hint" },
    sentence.slice(0, at),
    el("span", { class: "deletion-hint-word" }, word),
    sentence.slice(at + word.length),
  );
}

/** The gate row: the typed-`DELETE` field beside the button it unlocks. */
export function deletionGate({ hint, field, confirm }) {
  return el(
    "div",
    { class: "deletion-gate" },
    hint,
    el("div", { class: "deletion-gate-row" }, field, confirm),
  );
}

/** `Keep it — …`, the safe choice, full width and the strongest button in either column. */
export function keepButton({ label, onClick = null } = {}) {
  return button({
    kind: "primarySoft",
    size: "standard",
    label,
    onClick,
    padding: "10px",
    radius: "var(--r-9)",
    fontSize: "13px",
    class: "deletion-keep",
  });
}

/**
 * `Move to Proton's Trash` — the recoverable card's own action, and the quieter of its two buttons.
 *
 * A helper rather than a note telling the caller to pass `padding:"10px"`, because controls.js
 * writes padding INLINE and an inline style beats any rule `rows.css` could carry. There is no way
 * for the stylesheet to correct a caller who reaches for a plain `button()` here, so the only place
 * the geometry can live is a builder. Same reason `keepButton` exists.
 */
export function trashButton({ label, onClick = null } = {}) {
  return button({
    kind: "decision",
    size: "standard",
    label,
    onClick,
    padding: "10px",
    radius: "var(--r-9)",
    fontSize: "13px",
  });
}
