// The folder selector (#102 phase 5c-1, brief 4.2 option A).
//
// A pill in the header, right of the product name, drawn ONLY WHEN THE DAEMON RUNS TWO FOLDERS OR MORE
// (decision D1/D2). Below that nothing here is built, and a person with one folder sees the app they
// had — that is a claim about two renderings of every frame, held by `fidelity:n1` and by the 22 frames
// that pin the header's `flex:1` spacer. It names the folder the window is about; opened, it lists the
// folders with the word for each one's state and a neutral count of what is waiting in it, and choosing
// one is `select_pair`. It has no pause control of its own: one place per surface, the hero's button
// and the tray's row.
//
// THE MARKER IS NEVER A NUMBER (decision D4, `02-shell.md`: "putting a number in navigation is what the
// v1 sidebar did"), and it has TWO FORMS, because the pill can be asked two different things about the
// folders that are not on screen. The status chip and the attention band count the SELECTED folder only,
// because the screens under them act on one folder; the marker is how the other folders are not hidden
// behind that.
//
//   · THE RING says a person is needed: the same 6px ring `CHIP.decisions` draws, counting only what a
//     person must decide (conflicts, withheld deletions) in another folder, not a folder that is merely
//     busy.
//   · THE SOLID DOT says another folder has a PROBLEM: its last pass failed, or its folder is not there
//     (an unplugged drive — Rust derives `failed` for both, `gui_core::state::facts_of`). A failure in
//     the folder you are not looking at is otherwise a click away, behind a healthy-looking window (the
//     tray's rule: a failure is never hidden behind a healthy folder). Filled, in `--destructive`, the
//     hue the failed hexagon already draws; a hollow ring and a filled dot differ in shape as well as in
//     hue. A PAUSED folder is the person's own choice and does not mark the pill.
//
// When both apply the problem wins: it is the one a ring cannot say. The popover says which folder and,
// for what is waiting, how many. (Maintainer decision, #102, 2026-10-09.)
//
// BUILT ONCE AND PATCHED, for `app.js`'s reason (its `dom` comment): the header is patched on every ~2 s
// poll, and a pill or a row rebuilt there drops the keyboard to <body> inside two seconds. Everything
// the poll changes is a text node, a class or a data attribute here; `updatePairSelect` returns, and the
// popover's rows are patched by position. The popover is a CHILD of the pill's container and not a
// sibling of the header: `app.js`'s `setBody` anchors the screen on `header.nextSibling`, and a node
// standing there makes every poll answer "this block has moved" for every block and re-insert the
// whole screen, restarting the hexagon's animations (the ⋯ menu's comment records the same trap).
//
// The caret and the check are drawn as paths, not typed as `▾` and `✓`: neither is in the bundled faces
// (`fonts/README.md`), so a glyph would come from whichever fallback font the machine has and the box it
// measures would be the machine's rather than the design's.

import { el } from "./el.js";
import { CHROME } from "./copy.js";
import { fid } from "../fixtures/frames.js";

const SVG_NS = "http://www.w3.org/2000/svg";

function svgEl(tag, attrs = {}) {
  const node = document.createElementNS(SVG_NS, tag);
  for (const [key, value] of Object.entries(attrs)) node.setAttribute(key, String(value));
  return node;
}

/** The pill's `▾`, 8x8. */
function caret() {
  const svg = svgEl("svg", { viewBox: "0 0 8 8", class: "pair-pill-caret", "aria-hidden": "true" });
  svg.append(
    svgEl("path", {
      d: "M1 2.5 L4 5.5 L7 2.5",
      fill: "none",
      stroke: "var(--text-label)",
      "stroke-width": "1.2",
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
    }),
  );
  return svg;
}

/** The selected row's `✓`, 10x10. */
function check() {
  const svg = svgEl("svg", { viewBox: "0 0 10 10", class: "pair-row-check", "aria-hidden": "true" });
  svg.append(
    svgEl("path", {
      d: "M1.5 5.2 L4 7.5 L8.5 2.5",
      fill: "none",
      stroke: "var(--text-3)",
      "stroke-width": "1.3",
      "stroke-linecap": "round",
      "stroke-linejoin": "round",
    }),
  );
  return svg;
}

/**
 * The word for a derived state. Exhaustive over what `gui_core::DaemonState` serialises, with a last
 * arm that claims nothing: a state this build has never heard of is not `up to date` (#246's shape),
 * and is not a word either — the row names its folder and stops. `Object.hasOwn`, because a state is
 * wire text and `states["constructor"]` is a function.
 */
export function stateWordOf(state) {
  return Object.hasOwn(CHROME.pair.states, state) ? CHROME.pair.states[state] : "";
}

/**
 * The popover's rows, in the daemon's order (the first is the default folder).
 *
 * `pairStates` is Rust's own derivation per folder (`gui_core::state::pair_states`), read BY NAME into a
 * `Map` — a folder may be called `constructor`. A folder with no entry there has a `null` state and the
 * row draws no word, never `up to date`. `waiting` is asked of the caller because what counts
 * as waiting is the app's (it knows which deletions the person has already answered).
 *
 * `reachable` IS "THE LAST STATUS READ ANSWERED". When it did not, everything above is a memory: the
 * store keeps the last roster and the last derived states across a failed read (a payload with no reply
 * says nothing new), so a stopped daemon left every row saying `up to date` beside a chip that says
 * `unreachable` — #246's false all-clear, in a list. A row then says what the chip says, in the chip's
 * word, and counts nothing: whether a folder is waiting on a person is not known either, and a ring
 * drawn from the last answer would be a marker for a state nobody has seen since.
 */
export function selectorRows({
  pairs = [],
  pairStates = [],
  selected = null,
  waiting = () => 0,
  reachable = true,
} = {}) {
  const stateOf = new Map(pairStates.map((entry) => [entry.name, entry.state]));
  return pairs.map((pair) => ({
    name: pair.name,
    selected: pair.name === selected,
    state: reachable ? (stateOf.get(pair.name) ?? null) : "unreachable",
    waiting: reachable ? waiting(pair.name) : 0,
  }));
}

/** Does a folder OTHER than the selected one have something waiting on a person? */
export const othersWaiting = (rows) => rows.some((row) => !row.selected && row.waiting > 0);

/**
 * Has a folder OTHER than the selected one failed, or is it unavailable? One state, `failed`, because Rust
 * derives it for both (a pair whose folder is missing publishes its reason as `last_error`). Not `paused`
 * (the person's own choice), and not the process-wide `authExpired`/`unreachable`, which the chip and the
 * hero already say for the folder on screen and which every row would say at once.
 */
export const othersProblem = (rows) => rows.some((row) => !row.selected && row.state === "failed");

/** Which form the pill's marker takes, or `null`: the problem form outranks the ring when both apply. */
export function markerOf(rows) {
  if (othersProblem(rows)) return "problem";
  return othersWaiting(rows) ? "decision" : null;
}

/** The pill's accessible name: the folder, and what about another one is asking for attention. */
export function pillLabel(name, marker) {
  if (marker === "problem") return `Folder ${name}. Another folder has a problem.`;
  if (marker === "decision") return `Folder ${name}. Another folder has something waiting.`;
  return `Folder ${name}`;
}

// ------------------------------------------------------------------------------- the pill ----

/**
 * The marker, built exactly as the status chip builds its dots: the decision RING is transparent with a
 * 1px stroke, the problem form is a solid fill (`CHIP.deletions`' shape in the failed hexagon's hue).
 * `data-marker` says which form a node is, so a patch can tell a ring that must become a dot.
 */
function markerDot(kind) {
  const dot = el("span", { class: "chip-dot pair-pill-marker", "data-marker": kind });
  if (kind === "problem") dot.style.background = "var(--destructive)";
  else dot.style.border = "1px solid var(--decision)";
  return dot;
}

/**
 * The handlers are the CONTAINER'S, read when the event arrives — `pairSelect` stores them there and
 * `updatePairSelect` replaces them. A listener that closed over the object it was built with would keep
 * acting on the props of the first render for as long as the node lives, which is the whole life of a
 * pill nobody rebuilds (`main.js`'s `calledThrough` is the same lesson for the hero's buttons).
 */
const handlersOf = (event) => event.currentTarget.closest(".pair-select")?.__handlers ?? {};

function buildPill(name, marker) {
  const drawn = caret();
  fid(drawn.firstElementChild, "pillCaretPath");
  const pill = el(
    "button",
    {
      class: "pair-pill",
      type: "button",
      "aria-haspopup": "menu",
      "aria-expanded": "false",
      onClick: (event) => handlersOf(event).onToggle?.(),
    },
    marker ? fid(markerDot(marker), "pillMarker") : null,
    fid(el("span", { class: "pair-pill-name" }, name), "pillName"),
    fid(drawn, "pillCaret"),
  );
  return pill;
}

// ----------------------------------------------------------------------------- the popover ----

function buildRow(row) {
  const node = el(
    "button",
    {
      class: "pair-row",
      type: "button",
      role: "menuitemradio",
      // The folder is read off the node when the press arrives: rows are patched in place, so the
      // name a row was built with is not necessarily the one it shows.
      onClick: (event) => handlersOf(event).onPick?.(event.currentTarget.dataset.pair),
    },
    el("span", { class: "pair-row-lead" }, row.selected ? check() : null),
    el("span", { class: "pair-row-name" }),
    el("span", { class: "pair-row-state" }),
    el("span", { class: "pair-row-count" }),
  );
  return node;
}

/** Bring one row up to date in place. Touches only what differs, so a poll that changes nothing writes nothing. */
function patchRow(node, row) {
  if (node.dataset.pair !== row.name) node.dataset.pair = row.name;
  node.classList.toggle("is-selected", row.selected);
  node.setAttribute("aria-checked", row.selected ? "true" : "false");
  // The roving tab stop: one row is reachable by Tab, the arrows move among the rest.
  node.tabIndex = row.selected ? 0 : -1;
  const [lead, name, state, count] = node.children;
  if (Boolean(lead.firstChild) !== row.selected) lead.replaceChildren(...(row.selected ? [check()] : []));
  if (name.textContent !== row.name) name.textContent = row.name;
  const word = stateWordOf(row.state);
  if (state.textContent !== word) state.textContent = word;
  // Nothing waiting draws NOTHING, not `0 waiting`: a quiet folder is not a folder with a number.
  const waiting = row.waiting > 0 ? CHROME.chips.waiting(row.waiting) : "";
  if (count.textContent !== waiting) count.textContent = waiting;
}

function stampRow(node, index) {
  fid(node, "popoverRow", index);
  const [lead, name, state, count] = node.children;
  fid(lead, "rowLead", index);
  fid(lead.firstElementChild, "rowCheck", index);
  fid(lead.firstElementChild?.firstElementChild, "rowCheckPath", index);
  fid(name, "rowName", index);
  fid(state, "rowState", index);
  fid(count, "rowCount", index);
}

/** How many folders the list shows at full height before it scrolls (`.pair-popover.is-scrolling`: ten and a half rows). */
const ROWS_BEFORE_SCROLLING = 10;

function ensurePopover(container, rows) {
  let popover = container.querySelector(".pair-popover");
  if (!popover) {
    popover = el("div", {
      class: "pair-popover",
      role: "menu",
      "aria-label": "Folders",
    });
    container.append(fid(popover, "popover"));
  }
  // A list longer than the window can hold scrolls inside itself (`selector.css`). A class, and only for the
  // long list: the short one is the one the frames draw, and its computed `overflow` is compared.
  popover.classList.toggle("is-scrolling", rows.length > ROWS_BEFORE_SCROLLING);
  // Rows by position. Added and dropped at the end, never reordered: a reorder would move the node
  // under the keyboard.
  while (popover.children.length > rows.length) popover.lastElementChild.remove();
  while (popover.children.length < rows.length) {
    popover.append(buildRow(rows[popover.children.length]));
  }
  rows.forEach((row, index) => {
    const node = popover.children[index];
    patchRow(node, row);
    stampRow(node, index);
  });
  return popover;
}

// -------------------------------------------------------------------------- the whole control ----

/**
 * The control: a positioned container holding the pill and, while open, the popover.
 *
 * @param name      the selected folder
 * @param rows      `selectorRows(...)`
 * @param open      whether the popover is showing
 * @param handlers  `onToggle`, `onPick(name)`, `onKey(event)` (every key pressed on the pill or in the
 *                  popover) — read at the moment of an event, so a patch replaces them and nothing
 *                  has to rebind a node
 */
export function pairSelect({ name, rows, open, handlers = {} }) {
  const container = el("div", { class: "pair-select" });
  // One listener for the pill and for the popover's rows (arrows, Home, End, Esc): the rows come and go.
  container.addEventListener("keydown", (event) => event.currentTarget.__handlers?.onKey?.(event));
  container.append(fid(buildPill(name, markerOf(rows)), "pill"));
  fid(container, "pairSelect");
  return updatePairSelect(container, { name, rows, open, handlers });
}

/**
 * Patch the control across a poll. Never rebuilds the pill or a row: focus on either survives.
 * Returns the container, for the one caller that builds through it.
 */
export function updatePairSelect(container, { name, rows, open, handlers }) {
  if (handlers) container.__handlers = handlers;
  const marker = markerOf(rows);
  const pill = container.querySelector(".pair-pill");

  // The marker comes, goes and CHANGES FORM inside the pill: the button, and so the keyboard, stay where
  // they are. A ring that becomes a dot (or back) is the dot node replaced, never the pill.
  const dot = pill.querySelector(".pair-pill-marker");
  if (dot && dot.dataset.marker !== marker) dot.remove();
  if (marker && (!dot || dot.dataset.marker !== marker)) pill.prepend(fid(markerDot(marker), "pillMarker"));

  const label = pill.querySelector(".pair-pill-name");
  if (label.textContent !== name) label.textContent = name;
  pill.title = name;
  pill.setAttribute("aria-label", pillLabel(name, marker));
  pill.setAttribute("aria-expanded", open ? "true" : "false");

  if (open) ensurePopover(container, rows);
  else container.querySelector(".pair-popover")?.remove();
  return container;
}
