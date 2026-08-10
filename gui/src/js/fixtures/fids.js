// The node-key half of the fidelity mapping (F8/F9) — WHICH APP NODE STANDS FOR WHICH DRAWN NODE.
//
// Split out of frames.js when F9 gave every screen family its own data module. These tables describe
// the PROTOTYPE'S tree, not the app's data, and they are the one thing a data module may need from
// outside itself — so they live in a leaf with no imports at all, and nothing here can close a cycle
// back through `ui/`.
//
// `extract.mjs` derives a key per drawn node (`header/span[2]`, `div[2]/div[0]/span[1]`); nothing can
// derive which APP node corresponds, because the two trees differ by design — F4 wraps the app mark
// in a `<button>` the frames draw bare, and every screen diverges further. So each frame says it
// here, once, and `ui/chrome.js` and `ui/compact.js` stamp `data-fid` from it via `fid()`.
//
// A slot declared here that the frame has no node for is inert — `fid()` is only called where the
// screen renders that slot — but it is also a trap, because assert.mjs fails on a STAMPED key that
// does not exist. `check-fixtures.mjs` fails the build on a slot whose key exists in no frame that
// declares it, which is how the dead `hexRect`/`hexNumeral` declarations below were found.

/**
 * Where the shell's own slots live in each frame's node tree. The footer's key differs per frame
 * because the number of blocks above it does — `2a Settled` has one hero, `2a Needs you` has a hero
 * plus a transfer grid plus an attention band — which is exactly why this cannot be derived.
 */
export const SHELL_FIDS = {
  "2a Settled": {
    header: "header",
    mark: "header/img",
    name: "header/span[0]",
    spacer: "header/span[1]",
    chip: "header/span[2]",
    chipDot: "header/span[2]/span",
    menu: "header/button",
    footerNav: "div[2]",
    footerBar: "div[2]/div[0]",
    door: (i) => `div[2]/div[0]/span[${i}]`,
    footerLine: "div[2]/div[1]",
  },
  "2a Syncing": {
    header: "header",
    mark: "header/img",
    name: "header/span[0]",
    spacer: "header/span[1]",
    chip: "header/span[2]",
    chipDot: "header/span[2]/span",
    menu: "header/button",
    footerNav: "div[2]",
    footerBar: "div[2]/div[0]",
    door: (i) => `div[2]/div[0]/span[${i}]`,
    footerLine: "div[2]/div[1]",
  },
  "2a Needs you": {
    header: "header",
    mark: "header/img",
    name: "header/span[0]",
    spacer: "header/span[1]",
    chip: "header/span[2]",
    chipDot: "header/span[2]/span",
    menu: "header/button",
    footerNav: "div[3]",
    footerBar: "div[3]/div",
    door: (i) => `div[3]/div/span[${i}]`,
  },
};

/**
 * The compact panel's node keys (F6), for the eight in-scope dark frames.
 *
 * Written as a factory rather than as eight literal tables because the eight frames are four tree
 * SHAPES with different tails — and a factory states the correspondence once instead of eight times,
 * which is the form a reviewer can actually check. It is still hand-written: nothing derives which
 * app node stands for which drawn one, and this is where that judgement lives.
 *
 * Two rules the prototype's key scheme imposes, both of which produce silently-wrong maps if missed:
 *
 *   · AN INDEX APPEARS ONLY WHEN A TAG HAS SIBLINGS OF ITS OWN TAG. The settled mark's two paths are
 *     `path[0]`/`path[1]`; the needs-you mark's single path is `path`, with no index at all. Hence
 *     `HEX_PATHS`.
 *   · THE MENU'S SEPARATOR IS A CHILD LIKE ANY OTHER. `10a Settled` draws rows at `div[0..2]`, the
 *     rule at `div[3]`, and the last two rows at `div[4]`/`div[5]` — so rows are keyed by their
 *     position among all children, which is what `menuSection` stamps.
 */
const HEX_PATHS = {
  settled: 2,
  syncing: 3,
  needsYou: 1,
  deletions: 1,
  paused: 1,
  unreachable: 2,
  // The armed confirmation's 104px mark (`4a Armed`): the body outline, the warning bar, and a
  // separate `circle` for the dot. Two paths and not one, so the index has to appear.
  warning: 2,
};

/** The states whose mark carries a mono numeral — the three that have a count to show. */
const HEX_NUMERAL = new Set(["syncing", "needsYou", "deletions"]);

/**
 * The mark's own nodes, under whichever block holds it.
 *
 * The `defs` subtree is only worth mapping on the syncing mark — it is the only state with a
 * gradient — and only became worth mapping at all with #204, which put `stop-color`, `offset` and
 * `x1`/`y1`/`x2`/`y2` into the property lists. Note the prototype's tag is lower-cased in the key
 * (`lineargradient`), because `keyOf` writes `tagName.toLowerCase()`.
 */
function hexFids(under, state) {
  const idx = HEX_PATHS[state] > 1 ? (i) => `[${i}]` : () => "";
  const base = {
    hexagon: `${under}/svg`,
    hexPath: (i) => `${under}/svg/path${idx(i)}`,
    // A RECT AND A NUMERAL ARE PER-STATE, not part of every mark, and handing them to states that
    // draw neither is a mapping naming a node the frame does not have. Harmless while nothing
    // renders it — `fid()` no-ops on a slot the screen never reaches — and a trap the moment
    // something does, because assert.mjs then fails on a key that was wrong all along. Found by
    // check-fixtures.mjs, which is what that gate is for.
    //
    // `rect` is the paused mark's two bars and nothing else (hexagon.js's `svgEl("rect", …)` sits in
    // the paused branch alone). `text` is the count, which only the three states that HAVE a count
    // draw — verified against every compact frame's node list, not inferred from the component.
    ...(state === "paused" ? { hexRect: (i) => `${under}/svg/rect[${i}]` } : {}),
    // The warning mark's dot, and the only `circle` in the set — `unreachable` draws its strike as a
    // second path, not as a shape.
    ...(state === "warning" ? { hexCircle: `${under}/svg/circle` } : {}),
    ...(HEX_NUMERAL.has(state) ? { hexNumeral: `${under}/svg/text` } : {}),
  };
  if (state !== "syncing") return base;
  return {
    ...base,
    hexDefs: `${under}/svg/defs`,
    hexGradient: (i) => `${under}/svg/defs/lineargradient[${i}]`,
    hexStop: (i, j) => `${under}/svg/defs/lineargradient[${i}]/stop[${j}]`,
  };
}

/**
 * The main screen's node keys (S1), for the three in-scope `2a` windows.
 *
 * A factory for the same reason `compactFids` is: three frames, two tree shapes, and the differences
 * between them are a handful of parameters rather than three tables that happen to look alike. The
 * shell's own slots are NOT here — `SHELL_FIDS` already carries the header and footer per frame, and
 * `fixtures/main.js` spreads the two together, because which block the footer is depends on how many
 * blocks the screen put above it and only the frame knows that.
 *
 * The prototype's index-only-when-a-tag-has-siblings rule bites three times on this screen, and each
 * one is a silently-wrong map if missed:
 *
 *   · `2a Settled` draws two hero buttons (`button[0]`/`button[1]`); `2a Syncing` draws one, keyed
 *     `button` with no index at all. Hence `buttons`.
 *   · `2a Syncing`'s LEFT column holds an active row and a queued one (`div[0]`/`div[1]`); its right
 *     column and both of `2a Needs you`'s hold exactly one, keyed `div`. Hence `rowsInColumn`.
 *   · The settled mark has two paths and the syncing mark three, which `hexFids` already handles.
 *
 * @param state        "settled" | "syncing" — which mark and which hero arrangement
 * @param tail         "spacer" | "columns" — the empty flex:1 block, or the transfer grid
 * @param buttons      how many buttons in the hero's action row
 * @param column       "left" | "right" — which column the mapped transfer row is in
 * @param rowIndex     its position among that column's rows
 * @param rowsInColumn how many rows that column holds (1 means the key carries no index)
 * @param band         how many attention-band rows, or 0 for no band
 */
export function mainFids({
  state = "settled",
  tail = "spacer",
  buttons = 1,
  column = "left",
  rowIndex = 0,
  rowsInColumn = 1,
  band = 0,
} = {}) {
  const hero = "div[0]";
  const syncing = state === "syncing";
  // What sits ABOVE the headline inside the hero, and it is never nothing: settled has the glow at
  // `div[0]`, syncing has the seam and the two side-label blocks at `div[0..2]`. So the headline is
  // `div[1]` on `2a Settled` and `div[3]` on `2a Syncing`, and the sub-line and the action row
  // follow it. Getting this off by one is not loud — `div[0]/div[1]` exists in BOTH frames — so the
  // first version silently mapped the headline onto the glow and was caught two slots later, by
  // `check-fixtures.mjs`, on the only key that fell off the end of the tree.
  const above = syncing ? 3 : 1;
  const at = (i) => `${hero}/div[${i + above}]`;
  const actions = at(2);

  const map = {
    hero,
    ...hexFids(hero, state),
    headline: at(0),
    sub: at(1),
    actions,
    action: buttons > 1 ? (i) => `${actions}/button[${i}]` : () => `${actions}/button`,
    ...(syncing
      ? {
          seam: `${hero}/div[0]`,
          sideLocal: `${hero}/div[1]`,
          sideLocalLabel: `${hero}/div[1]/div[0]`,
          sideLocalPath: `${hero}/div[1]/div[1]`,
          sideRemote: `${hero}/div[2]`,
          sideRemoteLabel: `${hero}/div[2]/div[0]`,
          sideRemotePath: `${hero}/div[2]/div[1]`,
        }
      : { glow: `${hero}/div[0]` }),
  };

  if (tail === "columns") {
    const col = `div[1]/div[${column === "right" ? 1 : 0}]`;
    const row = rowsInColumn > 1 ? `${col}/div[${rowIndex}]` : `${col}/div`;
    Object.assign(map, {
      columns: "div[1]",
      columnLeft: "div[1]/div[0]",
      columnRight: "div[1]/div[1]",
      transferRow: row,
      // The main screen's ACTIVE row wraps its three spans in a flex body and hangs the 2px track
      // beside it; its queued row is flat, and so is every row in the compact panel. Measured, not
      // generalised — DEVIATIONS §65.
      transferBody: `${row}/div[0]`,
      transferName: `${row}/div[0]/span[0]`,
      transferDetail: `${row}/div[0]/span[1]`,
      transferArrow: `${row}/div[0]/span[2]`,
      transferTrack: `${row}/div[1]`,
      transferFill: `${row}/div[1]/div`,
    });
  } else {
    map.spacer = "div[1]";
  }

  if (band > 0) {
    const wrap = tail === "columns" ? "div[2]" : "div[1]";
    // One row is `div`; two or more are `div[0]`, `div[1]`. The same rule as everywhere else here,
    // and the reason `band` is a count rather than a boolean.
    const item = band > 1 ? (i) => `${wrap}/div/div[${i}]` : () => `${wrap}/div/div`;
    Object.assign(map, {
      bandWrap: wrap,
      band: `${wrap}/div`,
      bandItem: item,
      bandDot: (i) => `${item(i)}/span`,
      bandBody: (i) => `${item(i)}/div`,
      bandTitle: (i) => `${item(i)}/div/div[0]`,
      bandNote: (i) => `${item(i)}/div/div[1]`,
      bandAction: (i) => `${item(i)}/button`,
    });
  }

  return map;
}

/**
 * @param state    which of the six arrangements
 * @param tail     "footer" | "menu" — the block at the bottom, and its index in the panel
 * @param tailAt   how many blocks sit above the tail
 * @param buttons  how many footer buttons (a lone one is `button`, two are `button[0]`/`button[1]`)
 * @param rows     the transfer rows' directions, in order
 */
export function compactFids({ state, tail, tailAt, buttons = 0, rows = [] }) {
  const at = `div[${tailAt}]`;
  const btn = buttons > 1 ? (i) => `${at}/button[${i}]` : () => `${at}/button`;
  const map = {
    root: "",
    hero: "div[0]",
    [tail === "menu" ? "menu" : "footer"]: at,
  };

  if (state === "syncing") {
    // The seam turns the hero into a block holding three things, and the mark moves a level down.
    Object.assign(map, hexFids("div[0]/div[2]", state), {
      seam: "div[0]/div[0]",
      labels: "div[0]/div[1]",
      labelLocal: "div[0]/div[1]/span[0]",
      labelRemote: "div[0]/div[1]/span[1]",
      heroBody: "div[0]/div[2]",
      headline: "div[0]/div[2]/div",
      transfers: "div[1]",
      transferRow: (i) => `div[1]/div[${i}]`,
      // The arrow leads on an arriving row and trails on a leaving one (rows.js `transferSlotOrder`),
      // so which span is the name is a fact about the row's direction, not about its position.
      transferName: (i) => `div[1]/div[${i}]/span[${rows[i] === "down" ? 1 : 0}]`,
      transferArrow: (i) => `div[1]/div[${i}]/span[${rows[i] === "down" ? 0 : 1}]`,
      transferTrack: (i) => `div[1]/div[${i}]/div`,
      transferFill: (i) => `div[1]/div[${i}]/div/div`,
    });
  } else if (state === "deletions") {
    Object.assign(map, hexFids("div[0]", state), {
      headline: "div[0]/div",
      deletions: "div[1]",
      deletionRow: (i) => `div[1]/div[${i}]`,
      deletionHead: (i) => `div[1]/div[${i}]/div[0]`,
      deletionDot: (i) => `div[1]/div[${i}]/div[0]/span[0]`,
      deletionName: (i) => `div[1]/div[${i}]/div[0]/span[1]`,
      deletionNote: (i) => `div[1]/div[${i}]/div[1]`,
      actionBlock: "div[2]",
      actionButton: "div[2]/button",
    });
  } else {
    Object.assign(map, hexFids("div[0]", state), {
      headline: "div[0]/div[0]",
      sub: "div[0]/div[1]",
      subBreak: () => "div[0]/div[1]/br",
      meta: "div[0]/div[2]",
      action: "div[0]/button",
    });
  }

  if (tail === "menu") {
    Object.assign(map, {
      menuRow: (i) => `${at}/div[${i}]`,
      menuSep: (i) => `${at}/div[${i}]`,
      menuLabel: (i) => `${at}/div[${i}]/span[0]`,
      menuSub: (i) => `${at}/div[${i}]/span[1]`,
    });
  } else {
    Object.assign(map, {
      footerStatus: `${at}/span[0]`,
      footerSpacer: `${at}/span[1]`,
      footerButton: btn,
    });
  }
  return map;
}

// ------------------------------------------------------------------------- S2 · conflicts ----

/**
 * The shell slots on the three `3a` frames.
 *
 * SIMPLER THAN `2a`'s, in the one way that matters: there is no `footerLine`. The 2a frames draw the
 * footer's hairline as a SIBLING of the door bar (`div[2]/div[1]`); here the same rule is a
 * `border-top` on the bar itself, so declaring the slot would name a node that does not exist —
 * which `check-fixtures.mjs` fails the build over, and rightly.
 *
 * The cleared frame's header carries no chip and its name is the QUIET tier (#99A2AE against
 * #F2F4F7): nothing is waiting, so the header says so before the body does.
 */
const conflictShell = (tailAt, chip) => ({
  header: "header",
  mark: "header/img",
  name: "header/span[0]",
  spacer: "header/span[1]",
  ...(chip ? { chip: "header/span[2]", chipDot: "header/span[2]/span" } : {}),
  menu: "header/button",
  footerNav: `div[${tailAt}]`,
  footerBar: `div[${tailAt}]/div`,
  door: (i) => `div[${tailAt}]/div/span[${i}]`,
});

/**
 * The card view (`3a Conflict`) — one conflict filling the window.
 *
 * `card(i)` and friends take the SIDE as an index (0 = yours, 1 = Proton's) because the two columns
 * are the same shape drawn twice, and a table with `mineCard`/`theirsCard` pairs would let the two
 * drift. The screen passes 0 and 1 in the order it renders them, which is the order the grid puts
 * them in, which is the order the keys are numbered.
 */
const CONFLICT_CARD_FIDS = {
  titleRow: "div[0]",
  titleText: "div[0]/div[0]",
  title: "div[0]/div[0]/div[0]",
  sub: "div[0]/div[0]/div[1]",
  pager: "div[0]/div[1]",
  position: "div[0]/div[1]/span",
  pagerPrev: "div[0]/div[1]/button[0]",
  pagerNext: "div[0]/div[1]/button[1]",

  body: "div[1]",
  seam: "div[1]/div[0]",
  onSeam: "div[1]/div[1]",
  hexagon: "div[1]/div[1]/svg",
  path: "div[1]/div[1]/div[0]",
  onSeamMeta: "div[1]/div[1]/div[1]",

  cards: "div[1]/div[2]",
  cardCol: (i) => `div[1]/div[2]/div[${i}]`,
  cardEyebrow: (i) => `div[1]/div[2]/div[${i}]/div[0]`,
  card: (i) => `div[1]/div[2]/div[${i}]/div[1]`,
  cardHappened: (i) => `div[1]/div[2]/div[${i}]/div[1]/div[0]`,
  cardProse: (i) => `div[1]/div[2]/div[${i}]/div[1]/div[1]`,
  cardQuote: (i) => `div[1]/div[2]/div[${i}]/div[1]/div[1]/span`,
  cardMeta: (i) => `div[1]/div[2]/div[${i}]/div[1]/div[2]`,
  cardMetaItem: (i, j) => `div[1]/div[2]/div[${i}]/div[1]/div[2]/span[${j}]`,

  disclose: "div[1]/div[3]",
  discloseBtn: "div[1]/div[3]/button",

  choicesBlock: "div[2]",
  choices: "div[2]/div[0]",
  choice: (i) => `div[2]/div[0]/button[${i}]`,
  choiceRow: (i) => `div[2]/div[0]/button[${i}]/div[0]`,
  choiceGlyph: (i) => `div[2]/div[0]/button[${i}]/div[0]/span[0]`,
  choiceName: (i) => `div[2]/div[0]/button[${i}]/div[0]/span[1]`,
  choiceSub: (i) => `div[2]/div[0]/button[${i}]/div[1]`,
  choiceSubMono: (i) => `div[2]/div[0]/button[${i}]/div[1]/span`,

  note: "div[2]/div[1]",
  noteText: "div[2]/div[1]/span[0]",
  noteSpacer: "div[2]/div[1]/span[1]",
  later: "div[2]/div[1]/button",
};

/**
 * The diff view (`3a Conflict diff`).
 *
 * `diffCol` SKIPS A CHILD — 0 and 2, never 1 — because the panel's middle grid cell is the 1px
 * divider between the halves. Written as `i * 2` rather than as two named slots so the row keys
 * below can be built from the same index without a lookup.
 */
const CONFLICT_DIFF_FIDS = {
  diffHead: "div[0]",
  hexagon: "div[0]/svg",
  diffHeadText: "div[0]/div[0]",
  pathPlain: "div[0]/div[0]/div[0]",
  diffSummary: "div[0]/div[0]/div[1]",
  pager: "div[0]/div[1]",
  position: "div[0]/div[1]/span",
  pagerPrev: "div[0]/div[1]/button[0]",
  pagerNext: "div[0]/div[1]/button[1]",

  body: "div[1]",
  diffLabels: "div[1]/div[0]",
  diffLabel: (i) => `div[1]/div[0]/div[${i}]`,
  diffPanel: "div[1]/div[1]",
  diffCol: (i) => `div[1]/div[1]/div[${i * 2}]`,
  diffSplit: "div[1]/div[1]/div[1]",
  diffLine: (i, row) => `div[1]/div[1]/div[${i * 2}]/div[${row}]`,
  diffN: (i, row) => `div[1]/div[1]/div[${i * 2}]/div[${row}]/span[0]`,
  diffText: (i, row) => `div[1]/div[1]/div[${i * 2}]/div[${row}]/span[1]`,

  diffCounts: "div[1]/div[2]",
  diffCountsText: "div[1]/div[2]/span[0]",
  diffCountsSpacer: "div[1]/div[2]/span[1]",
  openBoth: "div[1]/div[2]/button[0]",
  hideDiff: "div[1]/div[2]/button[1]",

  queue: "div[1]/div[3]",
  queueEyebrow: "div[1]/div[3]/div[0]",
  queueRows: "div[1]/div[3]/div[1]",
  queueRow: (i) => `div[1]/div[3]/div[1]/div[${i}]`,
  queueDot: (i) => `div[1]/div[3]/div[1]/div[${i}]/span[0]`,
  queuePath: (i) => `div[1]/div[3]/div[1]/div[${i}]/span[1]`,
  queueReason: (i) => `div[1]/div[3]/div[1]/div[${i}]/span[2]`,
  queuePos: (i) => `div[1]/div[3]/div[1]/div[${i}]/span[3]`,
};

/** `3a Conflicts cleared` — one flat block, which is why the screen builds it flat. */
const CONFLICT_CLEARED_FIDS = {
  cleared: "div[0]",
  hexagon: "div[0]/svg",
  hexPath: (i) => `div[0]/svg/path[${i}]`,
  clearedTitle: "div[0]/div[0]",
  clearedSub: "div[0]/div[1]",
  clearedBack: "div[0]/button",
};

/** The three maps a `3a` fixture asks for by view. */
export function conflictFids(view) {
  const body = {
    card: CONFLICT_CARD_FIDS,
    diff: CONFLICT_DIFF_FIDS,
    cleared: CONFLICT_CLEARED_FIDS,
  }[view];
  if (!body) throw new Error(`fids: no conflict view "${view}"`);
  const tailAt = { card: 3, diff: 2, cleared: 1 }[view];
  return { ...conflictShell(tailAt, view !== "cleared"), ...body };
}

// ------------------------------------------------------------------------- S3 · deletions ----

/**
 * The shell slots on the two `4a` windows. Identical in shape to `conflictShell` — same header, same
 * doors, same absent `footerLine` — and separate from it because the two screens are free to move
 * apart, which is this file's rule for a table that describes a different frame.
 *
 * `4a Empty` takes none of it: the frame is a 522×422 standalone that draws neither the header nor
 * the doors, so declaring a slot for either would name a node it does not have — which
 * `check-fixtures.mjs` fails the build over.
 */
const deletionShell = (tailAt) => ({
  header: "header",
  mark: "header/img",
  name: "header/span[0]",
  spacer: "header/span[1]",
  chip: "header/span[2]",
  chipDot: "header/span[2]/span",
  menu: "header/button",
  footerNav: `div[${tailAt}]`,
  footerBar: `div[${tailAt}]/div`,
  door: (i) => `div[${tailAt}]/div/span[${i}]`,
});

/**
 * The queue (`4a Deletions`) — two severity columns either side of the seam.
 *
 * `c` IS THE COLUMN AND `i` IS THE CARD WITHIN IT, and the two indices are what make this table
 * survive a real queue: the frame draws one card per column and every card slot would work as a
 * fixed key at that arity, right up to the first user with two permanent deletions.
 *
 * Column 0 is permanent and column 1 recoverable, which is the drawn left-to-right order and also
 * the severity order — that coincidence is the screen's whole thesis, so the table numbers by
 * position and the screen decides which severity goes where.
 *
 * THE HEAD'S TWO SPANS SWAP. The dot sits on the OUTSIDE edge of each column, so it is `span[0]` on
 * the left and `span[1]` on the right (`rows.js` reverses the children). Keying `colDot` as
 * `span[0]` for both would map the right column's LABEL to the frame's dot — a 194px text node
 * against an 8px circle, reported as a size failure on a node that is perfectly correct.
 *
 * `card(c, i)` is `div[2 + i]`: the column's first two children are its eyebrow and its explanatory
 * sentence, and the cards follow.
 */
const card = (c, i) => `div[1]/div[1]/div[${c}]/div[${2 + i}]`;

const DELETION_QUEUE_FIDS = {
  titleRow: "div[0]",
  title: "div[0]/div[0]",
  sub: "div[0]/div[1]",

  body: "div[1]",
  seam: "div[1]/div[0]",
  columns: "div[1]/div[1]",
  column: (c) => `div[1]/div[1]/div[${c}]`,
  colHead: (c) => `div[1]/div[1]/div[${c}]/div[0]`,
  colDot: (c) => `div[1]/div[1]/div[${c}]/div[0]/span[${c === 0 ? 0 : 1}]`,
  colLabel: (c) => `div[1]/div[1]/div[${c}]/div[0]/span[${c === 0 ? 1 : 0}]`,
  colNote: (c) => `div[1]/div[1]/div[${c}]/div[1]`,

  card,
  cardTitle: (c, i) => `${card(c, i)}/div[0]`,
  cardName: (c, i) => `${card(c, i)}/div[0]/span[0]`,
  cardKind: (c, i) => `${card(c, i)}/div[0]/span[1]`,
  cardConsequence: (c, i) => `${card(c, i)}/div[1]`,
  cardEmphasis: (c, i) => `${card(c, i)}/div[1]/strong`,
  cardFacts: (c, i) => `${card(c, i)}/div[2]`,
  cardFact: (c, i, j) => `${card(c, i)}/div[2]/span[${j}]`,
  // The permanent card's fourth block is a gate and the recoverable card's is a button in a row.
  // Both are `div[3]` — the same position holding the thing that severity makes different — so they
  // are two slots over one key rather than one slot meaning two things, and only the column that
  // draws each ever stamps it.
  cardGate: (c, i) => `${card(c, i)}/div[3]`,
  gateHint: (c, i) => `${card(c, i)}/div[3]/div[0]`,
  gateWord: (c, i) => `${card(c, i)}/div[3]/div[0]/span`,
  gateRow: (c, i) => `${card(c, i)}/div[3]/div[1]`,
  gateField: (c, i) => `${card(c, i)}/div[3]/div[1]/input`,
  gateConfirm: (c, i) => `${card(c, i)}/div[3]/div[1]/button`,
  cardAction: (c, i) => `${card(c, i)}/div[3]`,
  actionButton: (c, i) => `${card(c, i)}/div[3]/button`,
  cardKeep: (c, i) => `${card(c, i)}/button`,

  queueFooter: "div[2]",
  footerRow: "div[2]/div",
  footerNote: "div[2]/div/span[0]",
  footerSpacer: "div[2]/div/span[1]",
  keepBoth: "div[2]/div/button",
};

/**
 * The armed confirmation (`4a Armed`) — the queue's body replaced, not a dialog over it.
 *
 * The word box is NOT an input. By the time this is up the word has already been typed on the card,
 * so the takeover restates it: a bordered `div` holding a mono span and a 1.5px caret span. Mapping
 * it to a second field would be the app growing a place to type that the design does not have.
 */
const DELETION_ARMED_FIDS = {
  armed: "div[0]",
  ...hexFids("div[0]", "warning"),
  armedTitle: "div[0]/div[0]",
  armedBody: "div[0]/div[1]",
  armedBodyPath: "div[0]/div[1]/span",
  armedRow: "div[0]/div[2]",
  armedWord: "div[0]/div[2]/div",
  armedWordText: "div[0]/div[2]/div/span[0]",
  armedCaret: "div[0]/div[2]/div/span[1]",
  armedConfirm: "div[0]/div[2]/button",
  armedKeep: "div[0]/button",
  armedCancel: "div[0]/div[3]",
};

/**
 * `4a Empty` — one flat block, and no shell at all.
 *
 * Keyed from `div` rather than `div[0]`: the frame's window has exactly one child, and the
 * prototype's key scheme only indexes a tag that has siblings of its own tag.
 */
const DELETION_EMPTY_FIDS = {
  empty: "div",
  ...hexFids("div", "settled"),
  emptyTitle: "div/div[0]",
  emptySub: "div/div[1]",
};

// ------------------------------------------------------------------------------ S4 · plan ----

/**
 * Shell slots for the three `5a` frames — two shapes, not one. `5a Plan` and `5a Plan safe` draw a
 * chip and the screen's own footer action bar (no doors, hence no `footerNav`/`door` slots).
 * `5a Checking` draws four doors and no chip; the app draws a chip there anyway (`06-plan.md`
 * Behaviour keeps `rehearsal · nothing has changed` up for the whole pass), so the slot is left
 * undeclared rather than compared against a node the frame does not have. DEVIATIONS §76.
 * No `chipDot` on any of them: the rehearsal chip is text only (`02-shell.md`, "none — text only").
 */
const planShell = (chip) => ({
  header: "header",
  mark: "header/img",
  name: "header/span[0]",
  spacer: "header/span[1]",
  ...(chip ? { chip: "header/span[2]" } : {}),
  menu: "header/button",
});

/**
 * The seam block both 1040 frames draw, at a different index in each. `s` is the drawn
 * left-to-right position (0 leaving, 1 arriving), not a direction — the screen decides what goes
 * where, as with S3's two columns.
 */
const planSides = (at) => ({
  seamBlock: at,
  seam: `${at}/div[0]`,
  sides: `${at}/div[1]`,
  side: (s) => `${at}/div[1]/div[${s}]`,
  sideLabel: (s) => `${at}/div[1]/div[${s}]/div[0]`,
  sideCount: (s) => `${at}/div[1]/div[${s}]/div[1]`,
  sideNumeral: (s) => `${at}/div[1]/div[${s}]/div[1]/span[0]`,
  sideUnit: (s) => `${at}/div[1]/div[${s}]/div[1]/span[1]`,
});

/**
 * `5a Plan` — the plan that would destroy something.
 *
 * Two frame slots are deliberately undeclared: `div[2]/div/button` (`Leave it alone`) and
 * `div[4]/button[0]` (`Run it without the deletion`) both need the filtered apply (G3, #192), and
 * `06-plan.md` says hide a button rather than fake it — so neither is drawn nor mapped.
 *
 * `run` is `button[1]`: with the frame's first button gone the app's button is still the frame's
 * second, and `button[0]` would compare a primary against a secondary.
 */
const PLAN_FIDS = {
  titleRow: "div[0]",
  titleText: "div[0]/div",
  title: "div[0]/div/div[0]",
  sub: "div[0]/div/div[1]",
  checkAgain: "div[0]/button",

  ...planSides("div[1]"),
  sideNote: (s) => `div[1]/div[1]/div[${s}]/div[2]`,

  bandWrap: "div[2]",
  band: "div[2]/div",
  bandMark: "div[2]/div/svg",
  bandMarkPath: (i) => `div[2]/div/svg/path[${i}]`,
  bandMarkDot: "div[2]/div/svg/circle",
  bandBody: "div[2]/div/div",
  bandTitle: "div[2]/div/div/div[0]",
  bandNote: "div[2]/div/div/div[1]",
  bandNotePath: "div[2]/div/div/div[1]/span",

  list: "div[3]",
  listHead: "div[3]/div[0]",
  listLabel: "div[3]/div[0]/span[0]",
  listSpacer: "div[3]/div[0]/span[1]",
  listCount: "div[3]/div[0]/span[2]",
  rows: "div[3]/div[1]",
  row: (i) => `div[3]/div[1]/div[${i}]`,
  rowGlyph: (i) => `div[3]/div[1]/div[${i}]/span[0]`,
  rowPath: (i) => `div[3]/div[1]/div[${i}]/span[1]`,
  rowOutcome: (i) => `div[3]/div[1]/div[${i}]/span[2]`,

  bar: "div[4]",
  gate: "div[4]/div",
  gateField: "div[4]/div/input",
  gateWhy: "div[4]/div/span",
  barSpacer: "div[4]/span",
  run: "div[4]/button[1]",
};

/** `5a Plan safe` — the hero, the two file lists, and the bar with both its buttons drawn. */
const PLAN_SAFE_FIDS = {
  hero: "div[0]",
  heroSeam: "div[0]/div[0]",
  heroMark: "div[0]/svg",
  heroMarkPath: (i) => `div[0]/svg/path[${i}]`,
  heroTitle: "div[0]/div[1]",
  heroSub: "div[0]/div[2]",

  ...planSides("div[1]"),
  sideList: (s) => `div[1]/div[1]/div[${s}]/div[2]`,
  sideRow: (s, i) => `div[1]/div[1]/div[${s}]/div[2]/div[${i}]`,
  sideRowGlyph: (s, i) => `div[1]/div[1]/div[${s}]/div[2]/div[${i}]/span[0]`,
  sideRowPath: (s, i) => `div[1]/div[1]/div[${s}]/div[2]/div[${i}]/span[1]`,
  sideRowNote: (s, i) => `div[1]/div[1]/div[${s}]/div[2]/div[${i}]/span[2]`,

  // `tailSpacer`, not `spacer`: `planShell` already declares `spacer` for the header's flex gap and
  // the two tables spread into one object, so a second `spacer` here wins and `renderHeader` stamps
  // the header's 0-height gap with this 116px block's key. The collision passes green — the box
  // comparison is skipped (the ⋯ taints the root's children) and both nodes set only
  // `flex-grow: 1; flex-basis: 0%`.
  tailSpacer: "div[2]",
  bar: "div[3]",
  checked: "div[3]/span[0]",
  barSpacer: "div[3]/span[1]",
  checkAgain: "div[3]/button[0]",
  run: "div[3]/button[1]",
};

/**
 * `5a Checking` — the rehearsal in flight. The mark is the syncing construction with F2's `dryRun`
 * dash, so it carries a gradient `defs` subtree the other two states have none of.
 * `div[0]/div[3]` (`8,431 of 12,480 files`) is undeclared: neither half of it has a source
 * (G9 #209, G7 #207) and the app draws no node for it.
 */
const PLAN_CHECKING_FIDS = {
  checking: "div[0]",
  checkingSeam: "div[0]/div[0]",
  checkingMark: "div[0]/svg",
  checkingMarkDefs: "div[0]/svg/defs",
  checkingMarkGradient: (i) => `div[0]/svg/defs/lineargradient[${i}]`,
  checkingMarkStop: (i, j) => `div[0]/svg/defs/lineargradient[${i}]/stop[${j}]`,
  checkingMarkPath: (i) => `div[0]/svg/path[${i}]`,
  checkingTitle: "div[0]/div[1]",
  checkingSub: "div[0]/div[2]",
  stop: "div[0]/button",

  footerNav: "div[1]",
  footerBar: "div[1]/div",
  // `door` IS UNDECLARED HERE, and it is the one slot in this file dropped because the PROTOTYPE is
  // wrong rather than because the app cannot draw it.
  //
  // `02-shell.md:42` states the rule without exception — "the active one is `#F2F4F7`" — and the
  // three S5 windows draw it: `Activity` lit, the other three at `#828B98`. `5a Checking` is the
  // plan screen, so `Plan a sync` should be lit and the frame paints all four unlit. S5 is what
  // surfaced it, because until then no mapped frame had a door that could be lit: `2a` is the root
  // and `3a`/`4a` are overlays, whose frames correctly light nothing.
  //
  // The app follows the prose. Mapping the doors here would assert the drawn mistake and turn a
  // correct screen into a red gate; a known-deviations row would be worse still, since that file's
  // bar is a MISSING CAPABILITY with an open issue and this is neither. DEVIATIONS §79.
};

/** The three maps a `5a` fixture asks for by view. */
export function planFids(view) {
  const body = {
    plan: PLAN_FIDS,
    safe: PLAN_SAFE_FIDS,
    checking: PLAN_CHECKING_FIDS,
  }[view];
  if (!body) throw new Error(`fids: no plan view "${view}"`);
  return { ...planShell(view !== "checking"), ...body };
}

/** The three maps a `4a` window fixture asks for by view. (`4a Compact` is F6's; see compactFids.) */
export function deletionFids(view) {
  const body = {
    queue: DELETION_QUEUE_FIDS,
    armed: DELETION_ARMED_FIDS,
    empty: DELETION_EMPTY_FIDS,
  }[view];
  if (!body) throw new Error(`fids: no deletion view "${view}"`);
  // The empty state is drawn as a standalone 522 surface with no chrome, so it gets no shell slots.
  return view === "empty" ? body : { ...deletionShell(view === "armed" ? 1 : 3), ...body };
}

// ------------------------------------------------------------------------------ S5 · activity ----
//
// SIX FRAMES, THREE TREE SHAPES, AND ONE THING TO WATCH: A KEY IS A NAME, NOT AN ADDRESS.
//
// assert.mjs resolves a stamped key against the FRAME's node map and compares that node's styles
// with the stamped element's. So where the app omits a block for want of data, the block below it
// keeps the key of the node it stands for even though its own position has moved — that is the
// mechanism working as designed (the same one that lets the footer's `<button class="door">` be
// compared against a drawn `span`), not a fiction about structure.
//
// Two frames of the same screen have INCOMPATIBLE shells. `7a Activity quiet` opens with a title
// block and puts the search field at `div[1]`; `7a File lookup` has no title node at all and the
// field IS `div[0]`. One table cannot describe both, so there are two.

/** The header, which all three windows draw identically. The chip is `idle` on every one of them. */
const activityShell = {
  header: "header",
  mark: "header/img",
  name: "header/span[0]",
  spacer: "header/span[1]",
  chip: "header/span[2]",
  chipDot: "header/span[2]/span",
  menu: "header/button",
};

/** The four doors, at whichever index the blocks above them leave. */
const activityDoors = (at) => ({
  footerNav: at,
  footerBar: `${at}/div`,
  door: (i) => `${at}/div/span[${i}]`,
});

/**
 * `7a Activity quiet` — the files tab with nothing waiting.
 *
 * UNDECLARED, AND EACH FOR A REASON THE SCREEN RECORDS: both seam sides' numeral rows
 * (`div[2]/div[2]/div[s]/div[1]`, G7 #207), the right side's `next full check in 4m`
 * (G4 #193), and the whole `Last things to move` head and its three rows
 * (`div[3]/div[1]/div[0..3]`, G13). The app draws no node for any of them.
 */
const ACTIVITY_QUIET_FIDS = {
  title: "div[0]/div[0]",
  sub: "div[0]/div[1]",

  searchWrap: "div[1]",
  search: "div[1]/div",
  searchIcon: "div[1]/div/span[0]",
  // The `<input>` against the drawn `span`. Styles, box and text are compared; the tag is recorded
  // and never compared — see `lookupField`.
  searchValue: "div[1]/div/span[1]",
  searchHint: "div[1]/div/span[2]",

  seamBlock: "div[2]",
  seam: "div[2]/div[0]",
  verdict: "div[2]/div[1]",
  hexagon: "div[2]/div[1]/svg",
  hexPath: (i) => `div[2]/div[1]/svg/path[${i}]`,
  agree: "div[2]/div[1]/div",
  sides: "div[2]/div[2]",
  sideLocal: "div[2]/div[2]/div[0]",
  sideRemote: "div[2]/div[2]/div[1]",

  content: "div[3]",
  // The warn band is TWO nodes here and one on `5a Plan`. `noticeBand({ wrapped: true })` is what
  // makes the outer a block with the padding and the inner the flex row.
  band: "div[3]/div[0]",
  bandRow: "div[3]/div[0]/div",
  bandGlyph: "div[3]/div[0]/div/span",
  bandBody: "div[3]/div[0]/div/div",
  bandTitle: "div[3]/div[0]/div/div/div[0]",
  bandNote: "div[3]/div[0]/div/div/div[1]",
  bandAction: "div[3]/div[0]/div/button",

  list: "div[3]/div[1]",
  listFoot: "div[3]/div[1]/div[4]",
  listNote: "div[3]/div[1]/div[4]/span[0]",
  // `All 7 files` (`button[0]`) is undeclared: it has no destination in any frame and no id in
  // routes.js, so the app draws only the tab switch beside it.
  passesButton: "div[3]/div[1]/div[4]/button[1]",

  ...activityDoors("div[4]"),
};

/**
 * `7a File lookup` — one path, resolved. No title block: the search field is the first content
 * block and wears the 4px padding-top the title block had.
 *
 * UNDECLARED: the four `This file's history` rows (`div[2]/div[0..3]`, G1 #190) and the two
 * openers in its footer row (`div[2]/div[5]/button[0..1]`, G14). The `linked · id` line stays,
 * because `proton_id` is on the reply today.
 */
const ACTIVITY_LOOKUP_FIDS = {
  searchWrap: "div[0]",
  search: "div[0]/div",
  searchIcon: "div[0]/div/span[0]",
  searchValue: "div[0]/div/span[1]",
  searchCount: "div[0]/div/span[2]",
  searchClear: "div[0]/div/button",

  seamBlock: "div[1]",
  seam: "div[1]/div[0]",
  hero: "div[1]/div[1]",
  hexagon: "div[1]/div[1]/svg",
  hexPath: (i) => `div[1]/div[1]/svg/path[${i}]`,
  lookupPath: "div[1]/div[1]/div[0]",
  lookupVerdict: "div[1]/div[1]/div[1]",
  lookupSub: "div[1]/div[1]/div[2]",

  cards: "div[1]/div[2]",
  card: (s) => `div[1]/div[2]/div[${s}]`,
  cardLabel: (s) => `div[1]/div[2]/div[${s}]/div[0]`,
  cardBox: (s) => `div[1]/div[2]/div[${s}]/div[1]`,
  cardMeta: (s) => `div[1]/div[2]/div[${s}]/div[1]/div[0]`,
  cardSize: (s) => `div[1]/div[2]/div[${s}]/div[1]/div[0]/span[0]`,
  cardPath: (s) => `div[1]/div[2]/div[${s}]/div[1]/div[1]`,

  content: "div[2]",
  historyFoot: "div[2]/div[5]",
  linked: "div[2]/div[5]/span[1]",

  ...activityDoors("div[3]"),
};

/**
 * `6a Activity passes` — the other tab. The pill strip exists ONLY here; the quiet tab's way in is
 * the button in its list footer.
 *
 * UNDECLARED: the whole twenty-bar chart card (`div[2]`, G12 — no per-pass duration exists) and
 * `Open the system log` (`div[3]/div[6]/button`, G14).
 */
const ACTIVITY_PASSES_FIDS = {
  title: "div[0]/div[0]",
  sub: "div[0]/div[1]",

  tabs: "div[1]",
  filesTab: "div[1]/button[0]",
  passesTab: "div[1]/button[1]",
  tabsSpacer: "div[1]/span",
  detailsButton: "div[1]/button[2]",

  passes: "div[3]",
  passRow: (i) => `div[3]/div[${i}]`,
  passesFoot: "div[3]/div[6]",
  retention: "div[3]/div[6]/span[0]",

  ...activityDoors("div[4]"),
};

/**
 * `7a Never synced` — the 602x602 dialog, one group instead of two.
 *
 * UNDECLARED: the whole `Can't be synced` group (`div[1]/div[4..7]`, G15). A socket or a symlink
 * never enters the index, so there is nothing to enumerate — which is a harder gap than the rule
 * group's was, not the same one twice.
 */
const NEVER_SYNCED_FIDS = {
  neverHead: "div[0]",
  neverHeadings: "div[0]/div",
  neverTitle: "div[0]/div/div[0]",
  neverSub: "div[0]/div/div[1]",
  neverClose: "div[0]/button",

  neverBody: "div[1]",
  ruleHeading: (i) => (i === 0 ? "div[1]/div[0]" : null),
  ruleSub: (i) => (i === 0 ? "div[1]/div[1]" : null),
  rulePattern: (i) => (i === 0 ? "div[1]/div[1]/span" : null),
  ruleRow: (i, j) => (i === 0 && j < 2 ? `div[1]/div[${j + 2}]` : null),
  ruleRowPath: (i, j) => (i === 0 && j < 2 ? `div[1]/div[${j + 2}]/span[0]` : null),
  ruleRowNote: (i, j) => (i === 0 && j < 2 ? `div[1]/div[${j + 2}]/span[1]` : null),
  changeRule: (i) => (i === 0 ? "div[1]/button" : null),

  neverFoot: "div[2]",
  reassurance: "div[2]/span[0]",
  done: "div[2]/button",
};

/**
 * `6a Details` — the 522x462 dialog of eight rows.
 *
 * UNDECLARED: `Open the system log` (`div[2]/button[1]`, G14). `Copy all` stays — the clipboard is
 * the webview's own and needs no command.
 */
const DETAILS_FIDS = {
  detailsHead: "div[0]",
  detailsTitle: "div[0]/div",
  detailsClose: "div[0]/button",

  detailsBody: "div[1]",
  kvRow: (i) => `div[1]/div[${i}]`,
  kvKey: (i) => `div[1]/div[${i}]/span[0]`,
  kvValue: (i) => `div[1]/div[${i}]/span[1]`,

  detailsFoot: "div[2]",
  copyAll: "div[2]/button[0]",
};

/**
 * `7a File pending` — 18 nodes, no title row, no ✕.
 *
 * UNDECLARED: the progress bar (`div[1]` and `div[1]/div`, G14's sibling — no fraction is
 * computable in either direction, see §63) and `Open folder` (`div[2]/button`, G14). Note the
 * bar's absence moves nothing: it sits BELOW the hero and above the footer row, and the footer row
 * keeps its own key.
 */
const FILE_PENDING_FIDS = {
  pendingHero: "div[0]",
  pendingHexagon: "div[0]/svg",
  pendingHexDefs: "div[0]/svg/defs",
  // NO INDEX on the gradient: this mark travels in ONE direction, so `defs` holds a single
  // `linearGradient` and the prototype's key scheme drops the index when a tag has no same-tag
  // sibling. The two-direction marks on `2a Syncing` carry `lineargradient[0]`/`[1]`.
  pendingHexGradient: "div[0]/svg/defs/lineargradient",
  pendingHexStop: (j) => `div[0]/svg/defs/lineargradient/stop[${j}]`,
  pendingHexPath: (i) => `div[0]/svg/path[${i}]`,
  pendingPath: "div[0]/div[0]",
  pendingTitle: "div[0]/div[1]",
  pendingSub: "div[0]/div[2]",

  pendingFoot: "div[2]",
  pendingNote: "div[2]/span[0]",
};

/**
 * The six maps an activity fixture asks for by view.
 *
 * EVERY DIALOG SLOT IS PREFIXED, and that is a collision rule rather than a naming preference. A
 * dialog floats over a body, so BOTH screens are rendering and both call `fid()`; a slot name in
 * two tables is resolved by NAME, so a `hexagon` declared for `7a File pending`'s 48px mark would
 * be stamped by whatever hexagon the screen behind it drew first — the 168px main-screen mark, or
 * the 52px settled mark on the quiet tab. That is the failure `activeRoute`'s own note describes,
 * one layer up, and it is reported as a size mismatch on a screen nobody was looking at.
 */
export function activityFids(view) {
  const body = {
    quiet: ACTIVITY_QUIET_FIDS,
    lookup: ACTIVITY_LOOKUP_FIDS,
    passes: ACTIVITY_PASSES_FIDS,
    neverSynced: NEVER_SYNCED_FIDS,
    details: DETAILS_FIDS,
    filePending: FILE_PENDING_FIDS,
  }[view];
  if (!body) throw new Error(`fids: no activity view "${view}"`);
  // The three dialogs are standalone surfaces with no app header and no footer doors — see
  // `deletionFids`, which draws the same line for `4a Empty`.
  const dialog = view === "neverSynced" || view === "details" || view === "filePending";
  return dialog ? body : { ...activityShell, ...body };
}
