// Per-frame fixtures (F9) — one deterministic dataset per in-scope frame label, selected by
// `?frame=<label>`.
//
// The same data drives the fidelity harness and the browser design preview, so a frame that passes
// CI is a frame a human can open and look at. That is the whole point: a gate whose inputs nobody
// can see is a gate nobody trusts.
//
// STATE OF PLAY. Every route is a placeholder until S1–S11 land, so a fixture can only feed what
// exists — today that is the shell F4 built. The three main-screen frames below are complete: their
// data drives the header and footer, and their `fids` map those nodes onto the drawn frame so
// assert.mjs has something real to compare. Each S-task adds its screens' rows.
//
// `fids` is the hand-written half of the mapping the F8 issue accepts as unavoidable. extract.mjs
// derives a key per drawn node (`header/span[2]`, `div[2]/div[0]/span[1]`); nothing can derive which
// APP node corresponds, because the two trees differ by design. So each frame says it here, once,
// and ui/chrome.js stamps `data-fid` from it.
//
// F6 added the eight compact frames. They are the first entries here that describe a WHOLE SURFACE
// rather than the shell's slots, because the compact panel is its whole frame — so they carry a
// `panel` key holding the arguments `ui/compact.js` takes, where the shell frames carry
// `status`/`conflicts`. app.js mounts whichever of the two a fixture describes.

import { MAIN, DELETIONS, TRAY } from "../ui/copy.js";

/** The frame the URL asks for, or null for the live app. */
export function activeFrame() {
  if (typeof location === "undefined") return null;
  return new URLSearchParams(location.search).get("frame");
}

/**
 * Where the shell's own slots live in each frame's node tree. The footer's key differs per frame
 * because the number of blocks above it does — `2a Settled` has one hero, `2a Needs you` has a hero
 * plus a transfer grid plus an attention band — which is exactly why this cannot be derived.
 */
const SHELL_FIDS = {
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
    hexRect: (i) => `${under}/svg/rect[${i}]`,
    hexNumeral: `${under}/svg/text`,
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
 * @param state    which of the six arrangements
 * @param tail     "footer" | "menu" — the block at the bottom, and its index in the panel
 * @param tailAt   how many blocks sit above the tail
 * @param buttons  how many footer buttons (a lone one is `button`, two are `button[0]`/`button[1]`)
 * @param rows     the transfer rows' directions, in order
 */
function compactFids({ state, tail, tailAt, buttons = 0, rows = [] }) {
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

/**
 * The datasets. Shaped exactly like the daemon's `StatusPayload` so the app cannot tell a fixture
 * from a live reply — anything that needs a special case here would be a special case in the app.
 *
 * The compact frames carry a `panel` instead: the arguments `ui/compact.js` takes. They are not a
 * status payload and should not be made into one — F6 ships the component, and deriving these from
 * a live status is S1's job for the window and S8's for the tray. A fixture that guessed at that
 * mapping now would be a third answer nobody had agreed to.
 */
const now = () => Math.floor(Date.now() / 1000);

export const FIXTURES = {
  "2a Settled": {
    fids: SHELL_FIDS["2a Settled"],
    status: {
      state: "idle",
      response: {
        status: "idle",
        paused: false,
        syncing: false,
        pending_changes: 0,
        last_sync_epoch_secs: now() - 120,
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    conflicts: [],
  },
  "2a Syncing": {
    fids: SHELL_FIDS["2a Syncing"],
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: now() - 14,
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    conflicts: [],
  },
  "2a Needs you": {
    fids: SHELL_FIDS["2a Needs you"],
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: now() - 14,
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    // Three decisions waiting: the chip reads `3 waiting` with the ring dot, which is what the
    // frame draws even though a transfer is also in flight (DEVIATIONS.md §44).
    conflicts: [{ path: "notes/todo.txt" }, { path: "docs/spec.md" }, { path: "a/b.txt" }],
  },

  // ---- the compact panel (F6). Eight frames, one component, two families. ----
  //
  // Every string that has a copy-deck entry comes from `ui/copy.js`; the rest — `2 minutes ago`,
  // `12,480 files`, `14s ago` — is formatter output, written literally here because a fixture must
  // reproduce the FRAME. Deriving them from `format.js` against a moving clock would make the gate's
  // input depend on when it ran, which is the one thing a fixture may not do.
  //
  // `menu: true` asks for the standard tray menu for this state (`TRAY_MENU` in ui/compact.js). It
  // is spelled as a flag rather than as the rows themselves because this file cannot import that
  // module: ui/compact.js imports `fid` from here, and `import-x/no-cycle` is an error.

  "2a Compact settled": {
    fids: compactFids({ state: "settled", tail: "footer", tailAt: 1, buttons: 2 }),
    panel: {
      state: "settled",
      headline: MAIN.compact.upToDate,
      sub: "2 minutes ago",
      subMono: true,
      footer: {
        status: "12,480 files",
        buttons: [{ label: MAIN.pause }, { label: MAIN.compact.open, kind: "secondaryAlt" }],
      },
    },
  },

  "2a Compact syncing": {
    fids: compactFids({
      state: "syncing",
      tail: "footer",
      tailAt: 2,
      buttons: 2,
      rows: ["up", "down"],
    }),
    panel: {
      state: "syncing",
      headline: MAIN.syncing(3),
      count: 3,
      // 0.64 and 0.31 of a 330px track — the two bars the frame draws, to the pixel.
      transfers: [
        { direction: "up", name: "docs/spec.md", progress: 0.64 },
        { direction: "down", name: "reports/q3-summary.pdf", progress: 0.31 },
      ],
      footer: {
        status: "14s ago",
        buttons: [{ label: MAIN.pause }, { label: MAIN.compact.open, kind: "secondaryAlt" }],
      },
    },
  },

  "2a Compact needs you": {
    fids: compactFids({ state: "needsYou", tail: "footer", tailAt: 1, buttons: 1 }),
    panel: {
      state: "needsYou",
      headline: MAIN.compact.needYou(3),
      count: 3,
      // Two sentences that break in a fixed place, not a paragraph that wraps.
      sub: [MAIN.compact.conflictLine, MAIN.compact.deletionLine],
      action: { label: MAIN.compact.review },
      footer: { status: MAIN.compact.syncingContinues, buttons: [{ label: MAIN.compact.later }] },
    },
  },

  "4a Compact": {
    fids: compactFids({ state: "deletions", tail: "footer", tailAt: 3, buttons: 1 }),
    panel: {
      state: "deletions",
      headline: DELETIONS.compact.title(2),
      count: 2,
      deletions: [
        { severity: "permanent", name: "photos/2019", note: DELETIONS.compact.permanent },
        { severity: "recoverable", name: "archive/old-notes.md", note: DELETIONS.compact.recoverable },
      ],
      // `Review them`, and nothing that approves anything — see ui/compact.js.
      action: { label: DELETIONS.compact.review },
      footer: { status: MAIN.compact.syncingContinues, buttons: [{ label: MAIN.compact.later }] },
    },
  },

  "10a Settled": {
    fids: compactFids({ state: "settled", tail: "menu", tailAt: 1 }),
    panel: {
      state: "settled",
      family: "tray",
      headline: MAIN.compact.upToDate,
      sub: "2 minutes ago · 12,480 files",
      subMono: true,
      menu: true,
    },
  },

  "10a Syncing": {
    fids: compactFids({ state: "syncing", tail: "menu", tailAt: 2, rows: ["up", "down"] }),
    panel: {
      state: "syncing",
      family: "tray",
      headline: MAIN.syncing(3),
      count: 3,
      transfers: [
        { direction: "up", name: "docs/spec.md", progress: 0.64 },
        { direction: "down", name: "reports/q3-summary.pdf", progress: 0.31 },
      ],
      menu: true,
    },
  },

  "10a Offline": {
    fids: compactFids({ state: "unreachable", tail: "menu", tailAt: 1 }),
    panel: {
      state: "unreachable",
      family: "tray",
      headline: TRAY.unreachableTitle,
      // Reassurance before the problem (voice rule 3), then the timing in a quieter tier.
      sub: TRAY.unreachableBody(4),
      meta: TRAY.retrying("40s", "13:58"),
      menu: true,
    },
  },

  "10a Paused": {
    fids: compactFids({ state: "paused", tail: "menu", tailAt: 1 }),
    panel: {
      state: "paused",
      family: "tray",
      headline: MAIN.paused,
      sub: MAIN.pausedSub(7, "13:20"),
      menu: true,
    },
  },
};

/**
 * THE THREE LIGHT TWINS ARE DELIBERATELY NOT MAPPED, and it is worth saying why here rather than
 * leaving them to look forgotten.
 *
 * They were mapped, run, and taken back out. The panel needs no new code in light — the same
 * fixture under `prefers-color-scheme: light` reproduces `12a Compact settled/syncing/needs light`
 * at every colour those frames actually declare. What it cannot reproduce is the colour they
 * INHERIT: the prototype draws all sixty frames on one dark page, so every node in a `12a` frame
 * that does not set a colour of its own inherits `#F2F4F7` from that page. The app in light mode
 * inherits `#14161A`, correctly, and fails on all 142 of them — 142 failures, one class, zero real.
 *
 * Making the gate right about this means recording, per node, whether the prototype set a property
 * or inherited it, which means regenerating all 51 fixtures. That is a change to the ground truth
 * and it belongs to S10, which owns light and needs the answer for the seven screens with no drawn
 * light frame at all. DEVIATIONS.md §58b carries the measurement so it starts from evidence.
 */

/** The fixture for the selected frame, or null. */
export function activeFixture() {
  const label = activeFrame();
  return label ? (FIXTURES[label] ?? null) : null;
}

/**
 * Stamp `data-fid` on a node, if a frame is selected and it has a key for this slot. A no-op in the
 * live app, so the attribute never ships to a user — it exists only for the harness and the preview.
 */
export function fid(node, slot, ...args) {
  const fixture = activeFixture();
  const key = fixture?.fids?.[slot];
  if (node && key != null) {
    node.setAttribute("data-fid", `${activeFrame()}:${typeof key === "function" ? key(...args) : key}`);
  }
  return node;
}
