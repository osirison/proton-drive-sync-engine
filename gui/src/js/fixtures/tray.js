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
import { compactFids } from "./fids.js";

export const TRAY_FIXTURES = {
  // The swatch sheet behind `10-tray.md` §"The glyph": two columns (`mono`, `colour`) × five rows,
  // one per state, each with the name and the sentence explaining the construction. Ten marks on one
  // page — which is also why DEVIATIONS §"gradient id" insists the syncing gradient's id be unique
  // per instance rather than a constant.
  "10a Glyph states": {
    specimen: {
      note: "the five 16px tray glyphs, mono and colour, drawn by ui/hexagon.js (F2) and shipped as symbolic SVGs by S8; the swatch card and its captions are the sheet, not product",
    },
  },

  // The GNOME top bar with our indicator open. The panel inside it is the needs-you compact panel
  // with the tray menu below it — the same component and the same state `2a Compact needs you`
  // carries, so the panel's arguments already exist and are not restated here.
  "10a In situ": {
    specimen: {
      note: "the tray panel (ui/compact.js, needs-you + menu) positioned under the indicator by S8; the 32px bar, the clock and the status cluster are a desktop mock and are never asserted",
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
