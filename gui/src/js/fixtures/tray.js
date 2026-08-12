// The system tray's datasets (F9) — `10a`, six frames that split two ways.
//
// FOUR ARE THE COMPACT PANEL IN A STATE. `10a Settled/Syncing/Offline/Paused` are `ui/compact.js`
// with the tray menu as its tail, so their fixture is that component's arguments and not a status
// payload. F6 wrote them when it built the component; F9 moved them here unchanged, so all six
// frames of a set are in the module named for it.
//
// TWO HAVE NO DATA AT ALL. `tools/fidelity/frame-classes.mjs` calls `10a Glyph states` and `10a In
// situ` `specimen`: a swatch sheet and a desktop mock, where the frame is scenery and only the inner
// artefact is product. Its `SPECIMEN_ARTEFACT` table says per frame what that artefact is and the
// harness asserts through it rather than against the whole frame — so a fixture supplying a
// wallpaper, a top bar or a clock would be supplying data for nodes nothing will ever check.
// Inventing a shape for them would be the more expensive mistake: `10a In situ` draws the needs-you
// panel, and a second answer to "what arguments make that panel" would sit in this very file
// contradicting `10a Settled` a few lines down.
//
// So each specimen is one line saying what the artefact is and who draws it, which is what the F9
// contract asks a specimen for. Nothing else belongs in them.

import { MAIN, TRAY } from "../ui/copy.js";
import { compactFids, glyphFids, without } from "./fids.js";

export const TRAY_FIXTURES = {
  // The swatch sheet behind `10-tray.md` §"The glyph": two columns (`mono`, `colour`) × five rows,
  // one per state, each with the name and the sentence explaining the construction. Ten marks on one
  // page — which is also why DEVIATIONS §"gradient id" insists the syncing gradient's id be unique
  // per instance rather than a constant.
  "10a Glyph states": {
    specimen: {
      note: "the five tray glyphs, mono and colour, drawn by ui/hexagon.js (F2) and shipped as symbolic SVGs by S8; the swatch card and its captions are the sheet, not product",
    },
    // The five forms IN THE ORDER THE SHEET LAYS THEM DOWN THE PAGE, which is what `glyphFids` needs
    // to key each cell — and which is ground truth about the drawing rather than about the
    // component. `ui/hexagon.js` names the same five in `TRAY_GLYPH_STATES`; the two are asserted
    // equal in `gui/test/tray.test.js` rather than one importing the other, because a fixture module
    // may not reach into `ui/` (see the header of frames.js) and because they are answers to two
    // different questions that happen to coincide.
    glyphs: ["settled", "syncing", "needsYou", "paused", "unreachable"],
    // 20, not the 16 the issue's title says. `10-tray.md` gives a RANGE ("rendered 15–20px") and all
    // ten marks on this sheet are drawn at exactly 20 — the range is what the desktop may scale the
    // SVG to, 20 is what the design measured. DEVIATIONS §82b.
    glyphSize: 20,
    fids: glyphFids(["settled", "syncing", "needsYou", "paused", "unreachable"]),
  },

  // The GNOME top bar with our indicator open. The panel inside it is `ui/compact.js` in its
  // needs-you state with the TRAY MENU as its tail — which is a combination no fixture carries:
  // `2a Compact needs you` is the same state with a footer instead, and the four panels above are
  // the menu tail in the other four states. So there is a real gap here, and leaving it is still
  // right: `frame-classes.mjs` classifies this frame `specimen` and asserts only the inner artefact,
  // so a panel written for it would be data nothing compares, invented to fill a slot. S8 draws it
  // and owns the arguments.
  "10a In situ": {
    specimen: {
      note: "the tray panel (ui/compact.js, needs-you + menu) positioned under the indicator by S8; the 32px bar, the clock and the status cluster are a desktop mock and are never asserted",
    },
    // S8 SUPPLIED THE ARGUMENTS THE NOTE ABOVE SAID IT WOULD. This is the combination no other
    // fixture carries: `2a Compact needs you` is the same state with a footer instead of the menu,
    // and the four `10a` panels are the menu with the other four states. So the gap the F9 note
    // recorded was real, and closing it needed the screen that owns the panel to exist.
    //
    // THE TOP BAR IS NOT MAPPED, and that is the specimen rule rather than an oversight.
    // `frame-classes.mjs` says the artefact here is "the compact panel over a desktop mock — assert
    // the panel, not the wallpaper". The bar does contain one piece of product — the 16px needs-you
    // glyph at `div[0]/span[3]/span[0]/svg`, our own indicator — but its construction is already
    // compared ten ways over on `10a Glyph states`, and claiming it here would quietly widen
    // `SPECIMEN_ARTEFACT` to mean something it does not say.
    // `meta` is dropped: this panel draws a headline, two sentences and a button, and no quieter
    // third line. The four unprefixed `10a` panels can declare it because the shared key resolves on
    // `10a Offline`, the one frame that draws one; a prefixed map has no sibling to resolve against.
    fids: without(
      compactFids({
        state: "needsYou",
        tail: "menu",
        tailAt: 1,
        // The panel is `div[1]`: `div[0]` is the GNOME top bar. Without this every key is off by a
        // level, and NOT loudly — `div[0]/svg` exists in this frame too (it is the indicator glyph),
        // so the 72px hero mark would have been compared against a 16px icon.
        prefix: "div[1]",
      }),
      "meta",
    ),
    panel: {
      state: "needsYou",
      family: "tray",
      headline: MAIN.compact.needYou(3),
      count: 3,
      // Two sentences that must break where the design put them, not where 362px happens to wrap.
      sub: [MAIN.compact.conflictLine, MAIN.compact.deletionLine],
      action: { label: MAIN.compact.review },
      menu: true,
    },
  },

  // ---- the four panel frames (F6). The tray panel IS ui/compact.js in a state, so these are
  // the component's arguments rather than a status payload. Written when F6 built the component
  // and moved here from frames.js by F9, unchanged, so all six 10a frames live in one module.
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
