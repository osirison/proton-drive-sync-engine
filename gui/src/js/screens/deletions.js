// The deletions screen (S3) — the approval queue, sorted by severity.
//
// `05-deletions.md`. The v1 screen listed both directions IDENTICALLY — same card, same red
// `Approve` button, same weight — with a paragraph on top trying to explain that one of them is
// permanent. That is the riskiest thing in the old UI, because the two are not equally dangerous:
// deleting on Proton goes to Proton Drive's Trash and can be undone; deleting locally leaves the
// disk with no trash and cannot. So the seam sorts by SEVERITY — permanent left, recoverable right
// — and everything else follows from that one decision.
//
// FOUR THINGS THIS MODULE OWNS.
//
//   · WHICH OF THE THREE BODIES IS SHOWING. Queue, armed takeover, and the empty state are one
//     screen. `bodyOf` is the whole decision, and armed outranks nothing — an empty queue makes the
//     takeover a window onto a deletion that is no longer waiting, so empty wins.
//   · WHICH COLUMN AN ITEM IS IN — asked of `rows.js`'s `severityOf`, which is the one place the
//     wire's `direction` becomes a severity, because the attention band asks it too. It is the
//     thing here that is easiest to get backwards: `DeleteDirection::Local` means "apply the delete
//     on the local disk", which is the PERMANENT column.
//   · WHAT THE CARD CAN SAY ABOUT WHAT YOU WOULD LOSE. `consequenceOf` and `factsOf`, both of which
//     have a Phase-1 shape and a Phase-2 shape, and neither of which invents a number.
//   · THAT THE GATE IS ONE PREDICATE. `gateSatisfied` is asked by the render and by the field's own
//     listener, so the button's disabled state and the arming decision cannot disagree.
//
// KEEP IS THE STRONGEST BUTTON IN BOTH COLUMNS, and — as on `3a Conflict` — that is a safety
// property rather than a styling choice. It is the reversible direction, so it wears the maximum
// contrast while the destructive one hides behind a typed word. There is NO cross-column
// `Approve all`: the only bulk action is `Keep both files`, and it is the safe one.
//
// WHAT PHASE 1 CANNOT DRAW, each recorded in DEVIATIONS.md §75 with the issue that closes it:
// the folder card's `1,204 photos, 8.4 GB` and the armed title's count (both the subtree aggregate,
// #208), `last opened Mar 2024` (an atime; the index stores mtime only, #208), `deleted on Proton
// 22m ago` (#225 — the field is re-stamped on every pass, so it is the age of the pass rather than
// of the deletion), and `Keep it` making the refusal durable or restoring the other side (#224 —
// the wire's `deny` revokes an approval and there is nothing that refuses one).

import { el } from "../ui/el.js";
import { DELETIONS } from "../ui/copy.js";
import { fileSize, monthYear } from "../ui/format.js";
import { renderHexagon } from "../ui/hexagon.js";
import { renderSeam } from "../ui/seam.js";
import { button, deleteGate, setButtonKind } from "../ui/controls.js";
import {
  deletionCard,
  deletionColumn,
  deletionGate,
  deleteHint,
  keepButton,
  severityOf,
  trashButton,
} from "../ui/rows.js";
import { fid } from "../fixtures/frames.js";

/**
 * The armed confirmation's mark, and the settled one on the empty state.
 *
 * 80px IS AMBIGUOUS AND F2 SAYS SO OUT LOUD: the design draws that size at 4.4 and at 4.6, so
 * `strokeForSize` refuses to pick and throws unless the caller names one. `4a Empty` draws 4.4 —
 * read off the frame's own `stroke-width`, not chosen — and passing it is the whole point of that
 * guard. Left off, the very first render of a fresh app throws inside `renderHexagon` and every
 * later render dies with it, which is a blank screen behind a working header.
 */
const ARMED_SIZE = 104;
const EMPTY_SIZE = 80;
const EMPTY_STROKE = 4.4;

/** The word, in the one place both the field and the button read it from. */
export const GATE_WORD = "DELETE";

/** The busy key for `Keep both files`, which is about the queue rather than about any one item. */
export const BULK_KEY = "all";

/**
 * The two columns, in drawn order. Index 0 is permanent and index 1 recoverable — which is left to
 * right AND most to least severe, and the fact that those two orders coincide is the screen's whole
 * argument rather than a coincidence to rely on quietly.
 */
const COLUMNS = ["permanent", "recoverable"];

// ------------------------------------------------------------------------------ the model ----

/**
 * The queue, split into its two columns in drawn order.
 *
 * A COLUMN WITH NOTHING IN IT IS NOT DRAWN, which is also how the settings policy gets honoured
 * without the screen knowing about it: on *only ask about permanent ones* the daemon never withholds
 * a Proton-side delete, so the recoverable column has no items and disappears — the same rule, not a
 * second one keyed off a config value the screen would then have to keep in step.
 */
export function splitQueue(items = []) {
  return COLUMNS.map((severity) => items.filter((item) => severityOf(item.direction) === severity));
}

/**
 * Which body to draw.
 *
 * EMPTY OUTRANKS ARMED. A queue that emptied while the takeover was up — the pass ran, or another
 * client approved it — leaves `armed` naming a deletion that is no longer waiting, and drawing the
 * confirmation for it would ask you to confirm something already gone. Asking for the item by
 * identity rather than trusting the flag is the same guard `bodyOf` makes on the conflicts screen.
 */
export function bodyOf({ items = [], armed = null } = {}) {
  if (!items.length) return "empty";
  return armedItem(items, armed) ? "armed" : "queue";
}

/**
 * The queued item `armed` names, or null when nothing in the queue answers to it.
 *
 * MATCHED ON THE FINGERPRINT AS WELL AS THE PATH, and the path alone was a live hazard rather than
 * a loose end. A confirmation is about one exact thing — the daemon pins its own approvals to the
 * fingerprint for the same reason — and a path is a slot that a later deletion can move into. Arm
 * `notes.txt`; the deletion resolves and drops out of the queue; the file comes back and is deleted
 * again with different content. Matching on the path alone re-binds the takeover to that NEW
 * deletion, and it comes back by itself with a live `Delete permanently` and no word typed.
 *
 * `armed` is therefore `{ path, fingerprint }` and not a string.
 */
export function armedItem(items = [], armed = null) {
  if (!armed?.path) return null;
  return (
    items.find(
      (item) =>
        item.path === armed.path &&
        item.fingerprint === armed.fingerprint &&
        severityOf(item.direction) === "permanent",
    ) ?? null
  );
}

/** The gate's one predicate. Case-sensitive: `delete` is a word people type by habit. */
export function gateSatisfied(value) {
  return value === GATE_WORD;
}

/** `(path, direction)` — the key the daemon's own approvals table uses, so the GUI cannot key differently. */
export function itemKey(item) {
  return `${item.path}\u0000${item.direction}`;
}

/**
 * What deleting this would cost you, and which words in it are the loss.
 *
 * ONE FUNCTION FOR BOTH HALVES. The sentence and the substring to emphasise have to agree — the
 * emphasis is a slice of the sentence — so they are decided together rather than by two callers who
 * happen to pick matching strings today. `splitEmphasis` already falls back to the whole sentence if
 * they ever stop matching; this is what stops them.
 *
 * The folder branch is the Phase-1 one. `DELETIONS.folderConsequence` needs the subtree aggregate
 * (#208) and nothing produces it, so the card says what is true without a number and emphasises
 * `everything inside it` — the loss, stated qualitatively, in the crimson the frame puts it in.
 */
export function consequenceOf(item) {
  if (severityOf(item.direction) === "recoverable") {
    return { sentence: DELETIONS.travelExplainer, emphasis: "Proton Drive's Trash" };
  }
  if (item.entity_kind === "directory") {
    return { sentence: DELETIONS.folderConsequenceUnknown, emphasis: "everything inside it" };
  }
  return { sentence: DELETIONS.fileConsequence, emphasis: "for good" };
}

/**
 * The mono facts strip: when it happened on the other side, and when you last touched it.
 *
 * BOTH OF THE FRAME'S FACTS ARE MISSING CAPABILITIES, which is why a folder card draws no strip.
 *
 *   · `deleted on Proton 22m ago` looks like it comes straight off `detected_epoch_secs`, and the
 *     field is not what its name suggests: `decide_delete_gate` stamps `now` on every withheld
 *     action and `self.pending_deletions` is replaced wholesale on every pass — which is forced to
 *     run while anything is pending, so an approval applies promptly. A deletion that happened
 *     three days ago therefore reports an age of seconds, refreshed every ~30s. That is not a
 *     rounding error in a fact, it is a different fact, on the screen whose job is telling you when
 *     something happened. Omitted, and filed as #225.
 *   · `last opened Mar 2024` is an atime and the index stores mtime only (#208).
 *
 * `last edited` is the mtime and is real, for a FILE — a directory record's mtime is the directory's
 * own, which is not when anything in it was edited, so it is left off there too. An em-dash would
 * be worse than an absence in every one of these: it claims the daemon was asked and did not know.
 */
export function factsOf(item, status) {
  if (item.entity_kind === "directory" || status?.mtime == null) return [];
  // `at: 1` IS THE DRAWN SLOT, not the DOM position, and the two have come apart. The frame's strip
  // is [when it was deleted, when you last touched it]; Phase 1 draws only the second, so the app's
  // first child stands for the frame's `span[1]`. Stamped by position it would be compared against
  // `deleted here 6m ago` — a node it is not — and reported as a width failure on a correct card.
  return [{ at: 1, text: DELETIONS.lastEdited(monthYear(status.mtime)) }];
}

/**
 * `a folder` / `4 KB` — the mono note beside the name.
 *
 * A directory says what it is; a file says how big it is, which is the more useful of the two and
 * the only one `path_sync_status` can answer. A file whose record has not arrived (or does not
 * exist) draws NOTHING rather than an em-dash: the slot is a fact about the file, and a dash there
 * reads as "zero bytes" on a card about losing it.
 */
export function kindNoteOf(item, status) {
  if (item.entity_kind === "directory") return "a folder";
  return status?.file_size == null ? null : fileSize(status.file_size);
}

// ------------------------------------------------------------------------------ the queue ----

function queueBody({ columns, statuses, busy, handlers, actions, gates }) {
  const body = fid(el("div", { class: "dl-body" }), "body");
  body.append(fid(renderSeam({ site: "deletionsList" }), "seam"));

  // One card per column is what fits, and what every frame draws; past that the grid scrolls rather
  // than pushing the footer off the bottom of a window that cannot grow. See deletions.css.
  const crowded = columns.some((items) => items.length > 1);
  const grid = fid(el("div", { class: "dl-columns" + (crowded ? " is-scrollable" : "") }), "columns");
  for (const [c, severity] of COLUMNS.entries()) {
    const items = columns[c];
    if (!items.length) continue;
    const column = fid(
      deletionColumn({
        severity,
        eyebrowText: severity === "permanent" ? DELETIONS.permanent : DELETIONS.recoverable,
        note: severity === "permanent" ? DELETIONS.permanentSub : DELETIONS.recoverableSub,
        cards: items.map((item, i) =>
          itemCard({ item, status: statuses.get(item.path), c, i, busy, handlers, actions, gates }),
        ),
      }),
      "column",
      c,
    );
    // Placed rather than flowed. A column is drawn where its SEVERITY says, so a queue holding only
    // recoverable deletions keeps its cards on Proton's side of the seam instead of sliding into
    // the permanent half — which is the one arrangement that would make the screen lie.
    column.style.gridColumn = String(c + 1);
    fid(column.querySelector(".deletion-head"), "colHead", c);
    fid(column.querySelectorAll(".deletion-head > *")[severity === "permanent" ? 0 : 1], "colDot", c);
    fid(column.querySelectorAll(".deletion-head > *")[severity === "permanent" ? 1 : 0], "colLabel", c);
    fid(column.querySelector(".deletion-policy"), "colNote", c);
    grid.append(column);
  }
  body.append(grid);
  return body;
}

/**
 * One waiting deletion, with the actions its severity allows.
 *
 * Pushes a `{ key, apply }` pair onto `actions`, which is how the busy state gets OUT of the render
 * signature — see `updateDeletions`. `apply(busy)` sets the same buttons a rebuild would have set,
 * without rebuilding: on this screen a rebuild is a half-typed `DELETE` destroyed, and clicking
 * `Move to Proton's Trash` on one card must not empty the gate on another.
 */
function itemCard({ item, status, c, i, busy, handlers, actions, gates }) {
  const permanent = severityOf(item.direction) === "permanent";
  const { sentence, emphasis } = consequenceOf(item);
  const facts = factsOf(item, status);
  const key = itemKey(item);
  let disabled = busy.has(key);
  const gate = permanent ? armGate({ item, isBusy: () => disabled, handlers, gates }) : null;
  // `disabled`, not merely a null handler. A null handler leaves the element fully live to the
  // pointer and the keyboard and simply inert, which is a click that appears not to have registered
  // — and it leaves the door open to a second one landing while the first is in flight. The paint
  // does not change (the design gives disabled its own KIND, and there is no drawn variant of an
  // in-flight `Keep it`); what changes is that the control stops accepting input, and the row
  // disappearing is the feedback.
  const trash = permanent
    ? null
    : trashButton({ label: DELETIONS.toTrash, disabled, onClick: () => handlers.onTrash(item) });
  const keep = keepButton({
    label: permanent ? DELETIONS.keepRemote : DELETIONS.keepLocal,
    disabled,
    onClick: () => handlers.onKeep(item),
  });
  actions.push({
    key,
    apply: (next) => {
      disabled = next;
      if (trash) trash.disabled = next;
      keep.disabled = next;
      gate?.repaint();
    },
  });

  const card = fid(
    deletionCard({
      name: item.path,
      kind: kindNoteOf(item, status),
      consequence: sentence,
      emphasis,
      facts: facts.map((fact) => fact.text),
      gate: gate?.node ?? null,
      action: trash,
      keep,
    }),
    "card",
    c,
    i,
  );

  fid(card.querySelector(".deletion-title"), "cardTitle", c, i);
  fid(card.querySelector(".deletion-name"), "cardName", c, i);
  fid(card.querySelector(".deletion-kind"), "cardKind", c, i);
  fid(card.querySelector(".deletion-consequence"), "cardConsequence", c, i);
  fid(card.querySelector(".deletion-emphasis"), "cardEmphasis", c, i);
  const strip = card.querySelector(".deletion-facts");
  fid(strip, "cardFacts", c, i);
  // Stamped from the FACT'S OWN SLOT rather than from its position in the DOM. The strip is built
  // from whatever Phase 1 can answer, so the app's first child is not the frame's first child — see
  // `factsOf`. The conflict card gets away with position because it only ever omits from the end.
  for (const [j, fact] of facts.entries()) {
    if (strip?.children[j]) fid(strip.children[j], "cardFact", c, i, fact.at);
  }
  if (permanent) {
    fid(card.querySelector(".deletion-gate"), "cardGate", c, i);
    fid(card.querySelector(".deletion-hint"), "gateHint", c, i);
    fid(card.querySelector(".deletion-hint-word"), "gateWord", c, i);
    fid(card.querySelector(".deletion-gate-row"), "gateRow", c, i);
    fid(card.querySelector(".delete-gate"), "gateField", c, i);
    fid(card.querySelector(".deletion-gate-row .btn"), "gateConfirm", c, i);
  } else {
    fid(card.querySelector(".deletion-action"), "cardAction", c, i);
    fid(card.querySelector(".deletion-action .btn"), "actionButton", c, i);
  }
  fid(card.querySelector(".deletion-keep"), "cardKeep", c, i);
  return card;
}

/**
 * The typed-`DELETE` gate on a permanent card: the hint, the field, and the button it unlocks.
 *
 * THE BUTTON'S STATE IS NOT RE-RENDERED. `disabled` is toggled on the live element from the field's
 * own listener, because a render here would go through `app.js` and rebuild the card the field is
 * being typed into. Both the listener and the click ask `gateSatisfied` — one predicate, so the
 * button cannot be enabled by a word the arming step then rejects.
 *
 * It reads the FIELD rather than a remembered value at click time for the same reason: the field is
 * the only thing that knows what is in it after `deletionGate`'s clear-on-blur has fired.
 */
function armGate({ item, isBusy, handlers, gates }) {
  // Geometry passed explicitly, because controls.js writes padding and radius INLINE and no rule in
  // rows.css can reach past that — the trap `keepButton`'s own comment records. The drawn button is
  // 10px 17px at radius 9, which is no rung.
  const confirm = button({
    kind: "destructiveDisabled",
    size: "standard",
    label: DELETIONS.delete,
    padding: "10px 17px",
    radius: "var(--r-9)",
    fontSize: "13px",
  });
  // ONE EXPRESSION FOR THE BUTTON'S STATE, asked by the field's listener and again whenever the
  // busy flag moves. Two rules — "the word matches" and "nothing is in flight" — evaluated in one
  // place, so a repaint from either side cannot contradict the other.
  const repaint = () =>
    setButtonKind(
      confirm,
      gateSatisfied(field.value) && !isBusy() ? "destructiveArmable" : "destructiveDisabled",
    );
  const field = deleteGate({ word: GATE_WORD, onChange: repaint });
  gates.push({ key: itemKey(item), field });
  confirm.addEventListener("click", () => {
    // Asked of the FIELD, not of a value remembered when the word matched: `deletionGate` clears the
    // field the moment focus leaves the pair, and the field is the only thing that knows.
    if (isBusy() || !gateSatisfied(field.value)) return;
    handlers.onArm(item);
  });
  return {
    node: deletionGate({ hint: deleteHint(DELETIONS.typeToDelete, GATE_WORD), field, confirm }),
    repaint,
  };
}

/** `Deletions stay here until you decide. Nothing expires.` and the one bulk action, which is safe. */
function queueFooter({ busy, handlers, actions }) {
  const keep = fid(
    button({
      kind: "primarySoft",
      size: "standard",
      label: DELETIONS.keepBoth,
      padding: "8px 15px",
      fontSize: "12.5px",
      disabled: busy.has(BULK_KEY),
      onClick: () => handlers.onKeepAll(),
    }),
    "keepBoth",
  );
  actions.push({ key: BULK_KEY, apply: (next) => (keep.disabled = next) });
  const row = fid(
    el(
      "div",
      { class: "dl-footer-row" },
      fid(el("span", { class: "dl-footer-note" }, DELETIONS.noExpiry), "footerNote"),
      fid(el("span", { class: "dl-spacer" }), "footerSpacer"),
      keep,
    ),
    "footerRow",
  );
  return fid(el("div", { class: "dl-queue-footer" }, row), "queueFooter");
}

// ----------------------------------------------------------------------------- the armed ----

/**
 * The full-window confirmation. A BODY SWAP, not a dialog (DEVIATIONS §57): the frame keeps the
 * header and the four doors and replaces only the content, so a scrim would be wrong.
 *
 * The word box is not a second field. You typed it on the card; this restates it, with the caret
 * blinking beside it — which is why the box is a `div` and the app grows no place to type here.
 */
function armedBody({ item, disabled, handlers, actions }) {
  const body = fid(el("div", { class: "dl-armed" }), "armed");
  const confirm = fid(
    button({
      kind: "destructive",
      size: "bar",
      label: DELETIONS.armedConfirm,
      padding: "12px 22px",
      radius: "var(--r-10)",
      fontSize: "13.5px",
      // The takeover gets `busy` too. Without it the one irreversible button in the app stays fully
      // live during its own round trip, so a slow socket invites a second click.
      disabled,
      onClick: () => handlers.onConfirmArmed(item),
    }),
    "armedConfirm",
  );
  const keep = fid(
    button({
      kind: "primarySoft",
      size: "standard",
      label: DELETIONS.keepRemote,
      padding: "10px 20px",
      radius: "var(--r-10)",
      fontSize: "13px",
      class: "dl-armed-keep",
      disabled,
      onClick: () => handlers.onKeep(item),
    }),
    "armedKeep",
  );
  actions.push({
    key: itemKey(item),
    apply: (next) => {
      confirm.disabled = next;
      keep.disabled = next;
    },
  });
  const mark = fid(renderHexagon({ size: ARMED_SIZE, state: "warning", tone: "destructive" }), "hexagon");
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "hexPath", i);
  fid(mark.querySelector("circle"), "hexCircle");

  const word = fid(
    el(
      "div",
      { class: "dl-armed-word" },
      fid(el("span", { class: "dl-armed-word-text" }, GATE_WORD), "armedWordText"),
      fid(el("span", { class: "dl-armed-caret", "aria-hidden": "true" }), "armedCaret"),
    ),
    "armedWord",
  );

  body.append(
    mark,
    // `armedTitle` takes the noun phrase, and Phase 1 has the path where the frame has
    // `1,204 photos` (#208). The question still names exactly what is about to go.
    fid(el("div", { class: "dl-armed-title" }, DELETIONS.armedTitle(item.path)), "armedTitle"),
    armedSentence(item),
    fid(el("div", { class: "dl-armed-row" }, word, confirm), "armedRow"),
    keep,
    fid(el("div", { class: "dl-armed-cancel" }, DELETIONS.armedCancel), "armedCancel"),
  );
  return body;
}

/**
 * `Everything in photos/2019 — 8.4 GB — is removed from disk. …`, with the path in inline mono.
 *
 * Split on the path rather than assembled from parts, the same trade `04-conflicts.md`'s quoted
 * line makes: the deck owns the whole sentence because the copy gate compares the whole sentence,
 * and the frame draws one span inside it that the copy cannot carry. `indexOf` and not a regex — a
 * path can contain anything a regex would read as syntax.
 */
function armedSentence(item) {
  // Folder grammar for a folder, file grammar for a file. `Everything in archive/old-notes.md` is
  // the frame's sentence applied to a thing that has no inside — and no frame draws a permanent
  // FILE, so nothing but this decides it.
  const render =
    item.entity_kind === "directory" ? (p) => DELETIONS.armedBody(p, null) : DELETIONS.armedBodyFile;
  const sentence = render(item.path);
  const node = el("div", { class: "dl-armed-body" });
  // WHERE THE TEMPLATE PUTS IT, not where the path first appears. `indexOf(path)` finds the first
  // textual match, and the sentence has words before the slot: a folder named `in` matched inside
  // `Everything`, and the mono span wrapped two letters of the first word. Rendering the same
  // template around a marker asks the deck itself where its hole is, so no path can be mistaken for
  // the prose around it. `\u0001` cannot occur in a path — and is written as an escape, because a
  // literal control character in a source file is its own bug (tools/check-sources.mjs).
  const at = render("\u0001").indexOf("\u0001");
  if (at < 0) node.textContent = sentence;
  else {
    node.append(
      sentence.slice(0, at),
      el("span", { class: "mono" }, item.path),
      sentence.slice(at + item.path.length),
    );
  }
  fid(node, "armedBody");
  fid(node.querySelector("span"), "armedBodyPath");
  return node;
}

// ----------------------------------------------------------------------------- the empty ----

/**
 * Nothing waiting.
 *
 * FLAT, with no inner wrapper, and drawn 522px wide where the shell is a fixed 1040 — the same two
 * facts as `3a Conflicts cleared`, and the same answer: a centred 520 column, which is the closest
 * the window can get, with the difference recorded rather than faked (#221, §75).
 */
function emptyBody() {
  const body = fid(el("div", { class: "dl-empty" }), "empty");
  const mark = fid(
    renderHexagon({ size: EMPTY_SIZE, state: "settled", strokeWidth: EMPTY_STROKE }),
    "hexagon",
  );
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "hexPath", i);
  body.append(
    mark,
    fid(el("div", { class: "dl-empty-title" }, DELETIONS.emptyTitle), "emptyTitle"),
    fid(el("div", { class: "dl-empty-sub" }, DELETIONS.emptySub), "emptySub"),
  );
  return body;
}

// ---------------------------------------------------------------------------- the screen ----

/**
 * What the last render was built from, so the next one can decide whether to build at all.
 *
 * THIS SCREEN MAY NOT BE REBUILT ON THE POLL, which is the one way it differs structurally from
 * S2. The conflicts screen rebuilds every pass because it has no state to protect; this one has a
 * focused `<input>` whose contents clear on blur BY DESIGN, so a rebuild every two seconds would
 * wipe a half-typed `DELETE` — and `14-behaviour-and-state.md` requires the gate to be completable
 * by keyboard. `dom`'s own comment in app.js names this exact failure a layer up.
 */
let view = null;

/**
 * Everything that changes the DOM, as one comparable string.
 *
 * WHAT THE CURRENT BODY DRAWS, and nothing else — which is a narrower thing than "the props", and
 * the difference matters most exactly where it is smallest. The armed takeover draws one path and
 * one button, so a `path_sync_status` reply landing for some other file in the queue must not
 * rebuild it: that reply is nothing the takeover shows, and the rebuild would drop the focus this
 * screen just placed on the safe button and restart the caret from zero. The empty state draws two
 * fixed sentences and depends on nothing at all.
 *
 * Everything a card draws is here, including the FINGERPRINT — a re-deletion of the same path with
 * different content is a different thing to be asked about, and a card that did not rebuild for it
 * would be showing the old question under the new one's identity.
 *
 * WITHOUT THE BUSY SET, which does change the DOM and is applied in place instead. It is the one
 * thing that moves while somebody is typing: click `Move to Proton's Trash` on one card and, folded
 * in here, the signature changes, the body rebuilds, and the half-typed `DELETE` in the OTHER card's
 * gate is gone. `updateDeletions` calls each card's `apply` instead, which sets exactly the buttons
 * a rebuild would have set.
 */
function signatureOf({ items, armed, statuses, body }) {
  if (body === "empty") return "empty";
  if (body === "armed") return JSON.stringify(["armed", armedItem(items, armed).path]);
  return JSON.stringify([
    "queue",
    items.map((item) => {
      const status = statuses.get(item.path);
      return [
        item.path,
        item.direction,
        item.entity_kind,
        item.fingerprint,
        status?.file_size ?? null,
        status?.mtime ?? null,
      ];
    }),
  ]);
}

/** The props every render path needs, normalised once. */
function viewOf(state) {
  const items = state.items ?? [];
  const statuses = state.statuses ?? new Map();
  const busy = state.busy ?? new Set();
  const armed = state.armed ?? null;
  return { items, statuses, busy, armed, body: bodyOf({ items, armed }) };
}

/**
 * Render the deletions screen as an ARRAY of window-root siblings — never a wrapper.
 *
 * Same rule as the conflicts screen: `shell.css` makes the window the flex column, and the seam's
 * `left: 50%` has to resolve against the 1040px window rather than against a wrapper.
 */
export function renderDeletions(state = {}) {
  const v = viewOf(state);
  const handlers = state.handlers ?? {};
  const typed = capturedGate();
  const actions = [];
  const gates = [];
  const nodes = buildBody(v, handlers, actions, gates);
  view = { sig: signatureOf(v), nodes, actions, gates };
  restoreGate(typed, gates);
  return nodes;
}

/**
 * What is in a gate right now, so a rebuild does not take it away.
 *
 * A REBUILD IS NOT ALWAYS AVOIDABLE, which is the half the busy-set change does not cover: a
 * `path_sync_status` reply landing for one card genuinely changes what that card draws, and the
 * signature has to notice. What must not happen is that the word somebody is halfway through typing
 * goes with it — the field clears on BLUR by design, and a rebuild is not a blur, so carrying it
 * across is honouring that rule rather than dodging it.
 *
 * Captured before the new body is built and while the old one is still attached, which is why it
 * lives here and not in the caller. At most one gate can be mid-word, so this returns the first it
 * finds rather than a list.
 */
function capturedGate() {
  for (const { key, field } of view?.gates ?? []) {
    const focused = field.ownerDocument?.activeElement === field;
    if (!focused && field.value === "") continue;
    return { key, value: field.value, focused, start: field.selectionStart, end: field.selectionEnd };
  }
  return null;
}

/**
 * Put it back on the card it belongs to, if that card is still there.
 *
 * KEYED, not positional: the rebuild may have been caused by the queue changing, and restoring a
 * typed `DELETE` onto whichever card happens to be first would arm a deletion nobody looked at.
 *
 * The value goes back through a dispatched `input` event rather than by calling the repaint
 * directly, so there is one path from "what the field holds" to "what the button looks like" and no
 * second one to keep in step. Focus is restored in a microtask because the nodes are not in the
 * document yet — `app.js` attaches them the moment this returns, and `focus()` on a detached node
 * is a silent no-op.
 */
function restoreGate(typed, gates) {
  if (!typed) return;
  const gate = gates.find((entry) => entry.key === typed.key);
  if (!gate) return;
  gate.field.value = typed.value;
  gate.field.dispatchEvent(new Event("input", { bubbles: true }));
  if (!typed.focused) return;
  queueMicrotask(() => {
    if (!gate.field.isConnected) return;
    gate.field.focus();
    gate.field.setSelectionRange(typed.start, typed.end);
  });
}

/**
 * The poll's path: rebuild only when something the body draws has moved, and otherwise touch
 * nothing at all.
 *
 * NOTHING ON THIS SCREEN COUNTS UP. An earlier version refreshed the facts strip's relative times
 * in place, because the frame draws `deleted on Proton 22m ago`; that clause is gone (#225 — the
 * field it would come from is re-stamped every pass), and the one fact left is a month. So the poll
 * is a comparison and nothing else, which is what the typed field needs from it.
 *
 * Returns the new blocks when it rebuilt and `null` when it did not, which is the shape `app.js`'s
 * main-screen branch already expects.
 */
export function updateDeletions(state = {}) {
  if (!view) return null;
  const v = viewOf(state);
  const sig = signatureOf(v);
  if (sig !== view.sig) return renderDeletions(state);
  for (const action of view.actions) action.apply(v.busy.has(action.key));
  return null;
}

/** Drop the cached view — the screen is going away, and the next mount must build from scratch. */
export function unmountDeletions() {
  view = null;
}

function buildBody(v, handlers, actions, gates) {
  if (v.body === "empty") return [emptyBody()];
  if (v.body === "armed") {
    const item = armedItem(v.items, v.armed);
    return [armedBody({ item, disabled: v.busy.has(itemKey(item)), handlers, actions })];
  }

  const titleRow = fid(
    el(
      "div",
      { class: "dl-title-row" },
      fid(el("div", { class: "dl-title" }, DELETIONS.title(v.items.length)), "title"),
      fid(el("div", { class: "dl-sub" }, DELETIONS.sub), "sub"),
    ),
    "titleRow",
  );
  return [
    titleRow,
    queueBody({
      columns: splitQueue(v.items),
      statuses: v.statuses,
      busy: v.busy,
      handlers,
      actions,
      gates,
    }),
    queueFooter({ busy: v.busy, handlers, actions }),
  ];
}
