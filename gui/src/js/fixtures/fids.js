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
const HEX_PATHS = { settled: 2, syncing: 3, needsYou: 1, deletions: 1, paused: 1, unreachable: 2 };

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
