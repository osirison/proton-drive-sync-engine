// The 51 in-scope frames and what KIND of thing each one is. IMPLEMENTATION-PLAN.md §1.2: they are
// not all windows, and asserting a content crop's width against 1040 is the first way a fidelity
// gate produces a failure that means nothing.
//
// Nine of the 60 drawn frames are out of scope — the `1a`/`1b`/`1c` round-one frames, and the two
// demoted tide-chart Activity frames. Listed rather than derived: "round one" is not a property of
// the markup.

export const OUT_OF_SCOPE = new Set([
  "1a Main",
  "1a Compact",
  "1b Main",
  "1b Compact",
  "1c Main",
  "1c Compact settled",
  "1c Compact attention",
  "6a Activity files",
  "6a Quiet",
]);

/**
 * `window`   — a full 1040×764 product window. Everything asserted, including its own box.
 * `dialog`   — a standalone window at its own size (`5a Checking` 522, `9a First sync` 602).
 * `compact`  — the 360px panel. Note it is drawn 362 overall: it opts OUT of border-box, and
 *              base.css opts everything in. DEVIATIONS.md §19 — F6 writes it as 362.
 * `crop`     — drawn at 600px but living inside the 1040 Settings window. Assert everything
 *              EXCEPT its own width, which is an artefact of how it was drawn.
 * `specimen` — a desktop mock or a swatch sheet. The frame is scenery; only the inner artefact is
 *              product. Asserted through `artefact`, never as a whole.
 */
export const FRAME_CLASS = {
  crop: ["8a Deletions tab", "8a Schedule monthly"],
  specimen: ["10a In situ", "11a In situ", "10a Glyph states", "12a Tray light"],
  // Desktop notification banners (S9). Their own class because the fit gate does not apply — a
  // banner is sized by the desktop's notification chrome, not by a 1040×764 window — and because
  // 520/600px would otherwise classify them as dialogs, which they are not.
  notification: ["11a Rules", "11a Settings", "11a Outage", "11a Grouped"],
};

/**
 * EVERY FRAMED SURFACE IS DRAWN 2px LARGER THAN ITS DECLARED SIZE, because the prototype does not
 * opt these nodes into `border-box` and adds a 1px border outside the declared width:
 *
 *   | surface        | declared  | drawn     |
 *   | window         | 1040×764  | 1042×766  |
 *   | 5a Checking    |  520×764  |  522×766  |
 *   | 6a Details     |  520×460  |  522×462  |
 *   | 9a First sync  |  600×540  |  602×542  |
 *   | compact panel  |  360×296  |  362×298  |
 *
 * DEVIATIONS.md §19 recorded this for the compact panel alone and told F6 to write 362. It is
 * general: `base.css` sets `box-sizing:border-box` globally, so an app surface written at its
 * NOMINAL size comes out 2px narrower than the frame. F5's dialog layer and F6's panel both have to
 * write the drawn number — or set `content-box` on that one element. Recorded as §48.
 */
export const BORDER_BOX_INSET = 2;

/**
 * For a specimen, the only thing worth asserting and where to find it. `10a Glyph states` and
 * `12a Tray light` are sheets of marks at tray size; the two `In situ` frames are the panel and the
 * notification sitting on a fake desktop, so the wallpaper, the taskbar and the clock are not
 * product and must not be asserted.
 */
export const SPECIMEN_ARTEFACT = {
  "10a In situ": "the compact panel over a desktop mock — assert the panel, not the wallpaper",
  "11a In situ": "the notification banner over a desktop mock",
  "10a Glyph states": "the tray glyphs themselves; the card behind them is a swatch sheet",
  "12a Tray light": "only the 14px glyph is product — its own card is drawn dark (DEVIATIONS §1.2)",
};

/** Width alone separates the remaining three classes, so those are derived rather than listed. */
export function classify(label, width) {
  for (const [kind, labels] of Object.entries(FRAME_CLASS)) {
    if (labels.includes(label)) return kind;
  }
  if (width <= 400) return "compact";
  if (width >= 1000) return "window";
  return "dialog";
}

/** Only a full window owes the fit gate a 1040×764 with nothing painting over the footer. */
export const OWES_FIT = (kind) => kind === "window";
