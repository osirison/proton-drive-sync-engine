// The browser design preview (F9) — the half of this task that is for people rather than for CI.
//
// The fidelity harness and this preview read the SAME fixtures, and that is the whole point of the
// pairing: a frame that passes the gate is a frame a human can open and look at, so "green" is
// falsifiable by eye. A gate whose inputs nobody can see is a gate nobody trusts.
//
// Three affordances, all gated on a query parameter Tauri never sets:
//
//   ?frames                 the index — every in-scope frame, linked
//   ?frame=<label>          that frame's dataset (api.js serves it to every command)
//   ?theme=light|dark       an explicit override, for looking at a light frame on a dark machine
//
// None of this ships to a user: the packaged app has no address bar, and every entry point below
// returns early when its parameter is absent. Same discipline as `fid()`.

import { FIXTURES, activeFixture, fid } from "./frames.js";
import { el } from "../ui/el.js";
import { trayGlyph } from "../ui/hexagon.js";
import { bannerFor, renderBanner } from "../ui/notification.js";

const params = () => new URLSearchParams(typeof location === "undefined" ? "" : location.search);

/**
 * THE THEME OVERRIDE IS EXPLICIT, AND NEVER INFERRED FROM THE FRAME LABEL. This is the one decision
 * in the preview worth arguing about, so here is the argument.
 *
 * The `12a` set IS the light theme, so it is tempting to have the preview notice a `12a` label and
 * set `data-theme="light"` itself. That would be wrong in a way that costs more than it saves.
 * `tokens.css` publishes the light palette TWICE — once under `@media (prefers-color-scheme: light)`
 * and once under `:root[data-theme="light"]` — because an explicit choice has to beat the media
 * query in both directions. `assert.mjs` pins the light frames with `emulateMediaFeatures`, which
 * exercises the FIRST block; auto-stamping the attribute would make the second block answer instead
 * and the media half could rot untested with the gate still green. A gate that supplies its own
 * answer is a gate agreeing with itself.
 *
 * So the override is a separate parameter a human types (or follows from the index, which links the
 * light frames with it), orthogonal to `?frame=`. `assert.mjs` never passes it, which is what keeps
 * the media pinning the only thing deciding the theme in CI.
 *
 * It is also deliberately NOT persisted. `toggleTheme` writes `localStorage.theme`, and a preview
 * that did the same would leak the last frame's theme into the next one — `assert.mjs` carries an
 * explicit clear-and-reload for exactly that hazard (see its note on localStorage surviving
 * navigation). An attribute with no write behind it cannot leak.
 */
export function previewTheme() {
  const value = params().get("theme");
  return value === "light" || value === "dark" ? value : null;
}

/** Apply the override, if one was asked for. Called from the app's own theme init. */
export function applyPreviewTheme() {
  const theme = previewTheme();
  if (theme) document.documentElement.setAttribute("data-theme", theme);
  return Boolean(theme);
}

/**
 * The frame sets, in the order the design bundle numbers them. Derived from the labels themselves
 * rather than from a second table: `frames/index.json` lives under `gui/tools/`, which is not served
 * to the app, and copying it into `src/` would be a second source of truth that could disagree with
 * the first. `check-fixtures.mjs` proves the registry's label set IS the index's, so grouping the
 * registry groups the frames.
 */
const SETS = [
  ["2a", "Main screen", "S1"],
  ["3a", "Conflicts", "S2"],
  ["4a", "Deletions", "S3"],
  ["5a", "Plan a sync", "S4"],
  ["6a", "Activity — passes and details", "S5"],
  ["7a", "Activity", "S5"],
  ["8a", "Settings", "S6"],
  ["9a", "Onboarding", "S7"],
  ["10a", "System tray", "S8"],
  ["11a", "Notifications", "S9"],
  ["12a", "Light theme", "S10"],
];

const setOf = (label) => label.split(" ")[0];

/**
 * The index page's own styling, injected when it mounts rather than linked from `index.html`.
 *
 * This page is developer chrome, not a design surface, and the two must not be confusable. A
 * `styles/preview.css` in the `<link>` chain would ship in every build, would be one more file
 * `index.html`'s comment warns you can silently forget, and would put preview furniture in the same
 * cascade as the design tokens the gate measures. Nothing here uses a token deliberately: if this
 * page ever looked like the app, someone would eventually screenshot it as one.
 */
const INDEX_CSS = `
.preview-index { font: 14px/1.5 system-ui, sans-serif; padding: 32px 40px 64px; max-width: 900px; }
.preview-index h1 { font-size: 20px; font-weight: 600; margin: 0 0 8px; }
.preview-index p { margin: 0 0 24px; opacity: .7; max-width: 62ch; }
.preview-index h2 { font-size: 13px; font-weight: 600; text-transform: uppercase;
  letter-spacing: .06em; opacity: .55; margin: 28px 0 8px; }
.preview-index .preview-task { float: right; font-weight: 400; opacity: .8; }
.preview-index ul { list-style: none; margin: 0; padding: 0;
  display: grid; grid-template-columns: repeat(auto-fill, minmax(240px, 1fr)); gap: 2px 16px; }
.preview-index a { color: inherit; }
`;

/**
 * The glyph sheet's own furniture (S8). Separate from `INDEX_CSS` because it is not the index, and
 * held to one rule the index is not: NOTHING HERE MAY SET A FONT OR A COLOUR THAT THE GLYPH CELLS
 * INHERIT.
 *
 * `base.css` sets exactly `font-family` and `color` on <body> and deliberately leaves `font-size`,
 * `line-height` and `letter-spacing` at the UA defaults, because the prototype's page sets exactly
 * those two — which is what makes an inherited value comparable at all. The gate records
 * `font-size: 16px`, `line-height: normal`, `text-align: start` and `color: rgb(242, 244, 247)` on
 * every one of the ten `<svg>` nodes, none of which the marks set themselves. So a `font-size` on
 * the grid, or a `text-align`, would fail ten nodes at once with a diff that points at the mark and
 * blames the wrong thing. The captions may style themselves; they are siblings, not ancestors.
 */
/**
 * The desktop under `11a In situ` (S9). Scenery, and `SPECIMEN_ARTEFACT` says so — the wallpaper,
 * the bar and the clock are not product and are not asserted.
 *
 * STYLED FROM TOKENS, NOT FROM THE PROTOTYPE'S LITERALS. The frame paints a GNOME-ish wallpaper in
 * three raw hexes; `check-tokens.mjs` allows a raw colour in `tokens.css` alone, and a fake
 * wallpaper is not a design token. What matters here is the STRUCTURE — the banners' fid keys are
 * positions inside this tree (`div[1]/div/div[0]`…), so the bar, the positioner and the 372px
 * column are load-bearing even though nothing compares them.
 */
const DESKTOP_CSS = `
.desktop-mock { width: 1040px; height: 560px; border-radius: 12px; overflow: hidden;
  box-shadow: var(--shadow-window); position: relative; background: var(--panel-alt); }
.desktop-bar { height: 32px; background: var(--surface); display: flex; align-items: center;
  padding: 0 14px; position: relative; }
.desktop-bar span { font-size: 11.5px; color: var(--text-2); }
.desktop-clock { position: absolute; left: 0; right: 0; text-align: center; font-weight: 600;
  color: var(--text-bright); pointer-events: none; }
.desktop-spacer { flex: 1; }
.desktop-stack { position: absolute; top: 44px; left: 0; right: 0; display: flex;
  flex-direction: column; align-items: center; gap: 10px; }
.desktop-column { width: 372px; display: flex; flex-direction: column; gap: 10px; }
`;

/**
 * `12a Tray light`'s scenery: a dark card holding a light strip, in a document that is light.
 *
 * IT DOES NOT MATCH THE FRAME'S SCENERY EXACTLY AND IT MUST NOT TRY. The card is drawn `#0E0F12` on
 * `#1A1D22` — dark values, in a light document, where no token carries them; the nearest light-palette
 * near-blacks are used instead. That is legal precisely because `SPECIMEN_ARTEFACT` narrows this frame
 * to the glyph: the card, the strip and the three status marks are compared by nothing, and buying an
 * exact card would cost either a raw colour (which `check-tokens.mjs` forbids outside `tokens.css`) or
 * a second copy of the dark palette scoped to a subtree, to paint a rectangle nobody measures.
 *
 * The DOCUMENT stays light, and that is the load-bearing half. The glyph takes `--hex-glyph-fg`,
 * which is `#14161A` under the light palette and `#E8EBF0` under the dark one — so the frame's whole
 * claim, that the mark inverts and needs nothing re-specified, is a fact about the theme the page is
 * in rather than about this file.
 */
const TRAY_LIGHT_CSS = `
.tray-light-card { width: 360px; box-sizing: border-box; padding: 18px 20px; border-radius: 12px;
  background: var(--btn-primary-bg); border: 1px solid var(--text-2); }
.tray-light-strip { height: 30px; background: var(--divider); border-radius: 7px; display: flex;
  align-items: center; padding: 0 11px; gap: 11px; }
/* PER TEXT SPAN, never on the strip or on every span in it. The indicator span sets no font-size in
   the frame, so its <svg> inherits the document's 16px — and the svg IS the mapped node, so a rule
   that reaches it makes the one thing this frame asserts fail on a property that has nothing to do
   with the glyph. It did: 10px against the frame's 16, three times. */
.tray-light-name { font-size: 10.5px; color: var(--text-3); }
.tray-light-mark { font-size: 10px; color: var(--text-3); }
.tray-light-spacer { flex: 1; }
.tray-light-indicator { display: inline-flex; align-items: center; padding: 3px 5px;
  border-radius: 5px; background: var(--border-strong); }
.tray-light-note { font-size: 12px; color: var(--text-5); margin-top: 14px; line-height: 1.55; }
`;

const GLYPH_CSS = `
.glyph-sheet { width: 560px; padding: 24px 26px; box-sizing: border-box; }
.glyph-grid { display: grid; grid-template-columns: 52px 52px 366px; align-items: center; gap: 0 18px; }
.glyph-rule { grid-column: 1 / -1; height: 1px; background: currentColor; opacity: .08; }
.glyph-cell { display: flex; justify-content: center; }
.glyph-head { font-size: 9.5px; text-transform: uppercase; letter-spacing: .14em; opacity: .5; }
.glyph-caption { padding: 14px 0; }
.glyph-caption b { display: block; font-size: 13px; font-weight: 600; }
.glyph-caption span { display: block; font-size: 12.5px; line-height: 1.5; opacity: .6; }
.glyph-foot { font-size: 12px; line-height: 1.6; opacity: .55; padding-top: 18px; max-width: 62ch; }
`;

/** `?frame=<label>` for this frame, plus the theme the light set wants. See `previewTheme`. */
function href(label) {
  const query = new URLSearchParams({ frame: label });
  if (setOf(label) === "12a") query.set("theme", "light");
  return `?${query}`;
}

/**
 * What each form is doing, for the human half of the pairing. Sheet prose, not product copy — it is
 * `10-tray.md`'s own "Notes" paragraph split per row, and it is why these are not in `ui/copy.js`
 * and not gated by `copy-gate.mjs`: nothing in the app ever renders them.
 */
const GLYPH_CAPTIONS = {
  settled: ["Up to date", "Hollow outline. The resting shape everything else is measured against."],
  syncing: ["Syncing", "A segment travels the edge. Motion reads at 16px where a colour shift doesn't."],
  needsYou: [
    "Needs you",
    "Filled centre — the only state that adds mass, so it's noticeable without being alarming.",
  ],
  paused: ["Paused", "Dashed and dimmed — the outline is interrupted, which is exactly what's happening."],
  unreachable: [
    "Can't reach Proton",
    "Struck through. Nothing is syncing and nothing is lost — it's waiting.",
  ],
};

/**
 * Stamp one mark's nodes. The shape differs per form — one path or two, a circle, a defs subtree —
 * so this reads what was drawn rather than being told, which is also what makes it honest: a form
 * that stopped emitting its circle stamps one fewer node and the gate reports the absence.
 */
function stampGlyph(svg, i) {
  fid(svg, "glyph", i);
  svg.querySelectorAll("path").forEach((path, j) => fid(path, "glyphPath", i, j));
  const circle = svg.querySelector("circle");
  if (circle) fid(circle, "glyphCircle", i);
  const defs = svg.querySelector("defs");
  if (!defs) return svg;
  fid(defs, "glyphDefs", i);
  const gradient = defs.querySelector("linearGradient");
  if (gradient) {
    fid(gradient, "glyphGradient", i);
    gradient.querySelectorAll("stop").forEach((stop, j) => fid(stop, "glyphStop", i, j));
  }
  return svg;
}

/**
 * `10a Glyph states` — the swatch sheet (S8).
 *
 * A SPECIMEN, so only the ten marks are product: `frame-classes.mjs` says so and its
 * `SPECIMEN_ARTEFACT` entry ("the tray glyphs themselves; the card behind them is a swatch sheet")
 * is what the harness asserts through. The grid, the rules and the captions exist here for two
 * reasons that are not "to be compared": a person opening `?frame=10a Glyph states` should see the
 * sheet the design drew, and — the load-bearing one — the marks' fid keys are positions inside that
 * grid (`div[0]/div[4]`, `div[0]/div[5]`, …). Drop the rule and the caption cells and every mark
 * after the first row lands on the wrong key, silently, because the neighbouring cell also holds an
 * `<svg>` with the same tag name.
 *
 * The two columns are the whole argument of the design: the LEFT one is every form at a single
 * colour. If a state is only distinguishable on the right, it is not a tray glyph.
 */
function renderGlyphSheet(root, label) {
  const fixture = activeFixture();
  const states = fixture?.glyphs ?? [];
  const size = fixture?.glyphSize ?? 20;

  const cells = [
    el("div", { class: "glyph-head" }, "mono"),
    el("div", { class: "glyph-head" }, "colour"),
    el("div", { class: "glyph-head" }, "what it means"),
  ];
  states.forEach((state, row) => {
    const [name, note] = GLYPH_CAPTIONS[state] ?? [state, ""];
    cells.push(el("div", { class: "glyph-rule" }));
    // Mono first, then colour — the order the sheet reads, and the order `glyphFids` keys.
    for (const mono of [true, false]) {
      cells.push(
        el(
          "div",
          { class: "glyph-cell" },
          stampGlyph(trayGlyph({ state, mono, size }), row * 2 + (mono ? 0 : 1)),
        ),
      );
    }
    cells.push(el("div", { class: "glyph-caption" }, el("b", {}, name), el("span", {}, note)));
  });

  root.replaceChildren(
    el(
      "div",
      { class: "glyph-sheet" },
      el("div", { class: "glyph-grid" }, ...cells),
      el(
        "div",
        { class: "glyph-foot" },
        "Every state is distinguishable in one colour, at 16px, on a light or dark panel. " +
          "Colour repeats the message for people who can use it.",
      ),
      el("div", { class: "glyph-foot" }, `${label} · ${states.length} forms, mono and colour`),
    ),
  );
}

/**
 * `11a In situ` — three banners over a desktop mock (S9).
 *
 * A SPECIMEN, so only the banners are product (`SPECIMEN_ARTEFACT`) and the bar, the clock and the
 * wallpaper are scenery — none of it carries a slot. But the scenery is load-bearing for the KEYS:
 * the banners' fids are positions inside this tree (`div[1]/div/div[0]`…), so dropping the top bar
 * or the positioner between the mock and the column moves every one of them onto a node the frame
 * does not have. `fids.js`'s `IN_SITU_BANNERS` is written against exactly this nesting.
 *
 * The strings on the bar are the prototype's own wallpaper furniture, which is why they are here and
 * not in `ui/copy.js`: nothing in the app ever renders them, and `copy-gate.mjs` would then be
 * checking a mock's clock.
 */
function renderDesktopMock(root, label) {
  const banners = activeFixture()?.desktop?.banners ?? [];
  root.replaceChildren(
    el(
      "div",
      { class: "desktop-mock" },
      el(
        "div",
        { class: "desktop-bar" },
        el("span", {}, "Activities"),
        el("span", { class: "desktop-clock" }, "Tue 14:41"),
        el("span", { class: "desktop-spacer" }),
        el("span", {}, `${label} · ${banners.length} banners`),
      ),
      el(
        "div",
        { class: "desktop-stack" },
        el(
          "div",
          { class: "desktop-column" },
          ...banners.map((banner, i) => renderBanner(bannerFor(banner.event), { at: banner.at, index: i })),
        ),
      ),
    ),
  );
}

/**
 * `12a Tray light` — one glyph on a light panel (S10).
 *
 * THE ONLY `12a` FRAME WHOSE CARD IS DRAWN DARK, which is the whole reason it is a specimen and not
 * a light compact panel: `#0E0F12`, radius 12, `padding:18px 20px`, holding a light GNOME strip and
 * a sentence about it. `SPECIMEN_ARTEFACT` narrows it to the 14px needs-you glyph — `stroke:#14161A`,
 * `stroke-width:9`, a filled `r=17` circle — and the issue that asks for this frame says to assert
 * that and nothing else.
 *
 * So everything below except the `<svg>` is scenery, and it exists for the two reasons the other two
 * specimens' scenery does: a person opening `?frame=12a Tray light` should see what the design drew,
 * and the glyph's key is a POSITION inside this tree (`div[0]/span[2]/svg`). Drop the spacer or one
 * of the three status glyphs after it and the mark lands on a node the frame does not have.
 *
 * The strip is drawn light while the card around it is dark, and no token can express that — a theme
 * is a property of the document here, not of a subtree — so the strip's own colours are the preview's
 * (`TRAY_LIGHT_CSS`), exactly as the desktop mock's wallpaper is. Nothing here ships.
 */
function renderTrayLight(root, label) {
  const { state = "needsYou", size = 14 } = activeFixture()?.trayStrip ?? {};
  root.replaceChildren(
    el(
      "div",
      { class: "tray-light-card" },
      el(
        "div",
        { class: "tray-light-strip" },
        el("span", { class: "tray-light-name" }, "Activities"),
        el("span", { class: "tray-light-spacer" }),
        // `mono`, and it is the point of the frame rather than a rendering choice: the glyph inverts
        // by taking `--hex-glyph-fg`, which is `#14161A` in light — so a light panel needs no second
        // drawing, only the theme it is already in. The strip is light, so the app is asked for its
        // light palette by `?theme=light`, which `href()` puts on every `12a` link.
        el("span", { class: "tray-light-indicator" }, stampGlyph(trayGlyph({ state, mono: true, size }), 0)),
        el("span", { class: "tray-light-mark" }, "▲"),
        el("span", { class: "tray-light-mark" }, "◐"),
        el("span", { class: "tray-light-mark" }, "▮"),
      ),
      el(
        "div",
        { class: "tray-light-note" },
        "The glyph inverts to near-black and keeps its five forms. Because state is carried by fill " +
          `rather than hue, nothing has to be re-specified for a light panel. · ${label}`,
      ),
    ),
  );
}

/**
 * The index. Rendered only for `?frames`, and it takes over the window — the shell never renders
 * behind it.
 *
 * It is NOT true that nothing polls behind it: `main()` starts the status poll unconditionally, and
 * an early return from `render()` does not undo that. `app.js` latches `dom.preview` so this runs
 * once instead of every ~2 s, which matters because a rebuilt list drops focus from a tabbed-to link.
 */
function renderIndex(root) {
  const labels = Object.keys(FIXTURES);
  const bySet = new Map(SETS.map(([prefix]) => [prefix, []]));
  const orphans = [];
  for (const label of labels.sort()) {
    const bucket = bySet.get(setOf(label));
    if (bucket) bucket.push(label);
    // A label whose prefix is not a known set is listed rather than dropped. It means someone added
    // a frame set without adding it here, and a silently missing frame is the failure this whole
    // page exists to prevent.
    else orphans.push(label);
  }

  const section = (title, task, items) =>
    el(
      "section",
      { class: "preview-set" },
      el("h2", {}, title, el("span", { class: "preview-task" }, task ?? "")),
      el(
        "ul",
        {},
        ...items.map((label) =>
          el("li", {}, el("a", { href: href(label) }, label), setOf(label) === "12a" ? " ☀" : ""),
        ),
      ),
    );

  root.replaceChildren(
    el(
      "div",
      { class: "preview-index" },
      el("h1", {}, `Design-v2 frames · ${labels.length} in scope`),
      el(
        "p",
        {},
        "The same datasets the fidelity gate runs on. A frame that passes CI is a frame you can " +
          "open here. Light frames carry an explicit theme override — see preview.js for why it is " +
          "not inferred.",
      ),
      ...SETS.filter(([prefix]) => bySet.get(prefix).length).map(([prefix, title, task]) =>
        section(`${prefix} · ${title}`, task, bySet.get(prefix)),
      ),
      ...(orphans.length ? [section("Ungrouped", null, orphans)] : []),
    ),
  );
}

/**
 * A `?frame=` label with no fixture behind it. Loud, because the alternative is what this used to
 * do: fall through to the generic mock and render a plausible screen that is not the frame you
 * asked for. A typo in a 24-character label ("2a Compact settled" vs "2a Compact Settled") would
 * otherwise look like a fidelity failure in the app.
 */
function renderUnknown(root, label) {
  const labels = Object.keys(FIXTURES);
  const wanted = label.toLowerCase();
  // Anything sharing the frame-set prefix, or differing only by case. Enough to spot a typo without
  // pulling in an edit-distance routine nothing else here needs.
  const near = labels.filter((l) => l.toLowerCase() === wanted || setOf(l) === setOf(label));
  root.replaceChildren(
    el(
      "div",
      { class: "preview-index" },
      el("h1", {}, "No fixture for that frame"),
      el("p", {}, `?frame=${label} does not match any of the ${labels.length} in-scope frame labels.`),
      ...(near.length
        ? [
            el("p", {}, "Did you mean:"),
            el("ul", {}, ...near.map((l) => el("li", {}, el("a", { href: href(l) }, l)))),
          ]
        : []),
      el("p", {}, el("a", { href: "?frames" }, "All frames")),
    ),
  );
}

/**
 * The preview's claim on the window, if it has one. Called first in `render()`; a `true` return
 * means the shell must not boot.
 *
 * Note what is NOT here: a frame that HAS a fixture renders through the ordinary app, with the
 * fixture reaching it through `api.js`'s mock. The preview must not become a second renderer — the
 * point is to look at the real screens, and a screen the preview drew specially would be a screen
 * the gate never checked.
 */
export function mountPreview(root) {
  const query = params();
  const label = query.get("frame");
  const claim = query.has("frames")
    ? renderIndex
    : label != null && !FIXTURES[label]
      ? renderUnknown
      : // A specimen sheet is a third claim on the window, and it is claimed the same way the other
        // two are: by the fixture saying so. `glyphs` is what says so — the sheet is the only frame
        // whose product is a set of marks rather than a screen, so there is nothing for the shell or
        // for `mountFramePanel` to draw and both would fall through to the generic mock.
        FIXTURES[label]?.glyphs
        ? renderGlyphSheet
        : // The fourth claim, and the same argument (S9): `11a In situ`'s product is three banners
          // over a desktop, which is neither a screen nor a panel — nothing in the shell draws one.
          FIXTURES[label]?.desktop
          ? renderDesktopMock
          : // The fifth, and the last one the bundle has (S10): `12a Tray light` is one glyph on a
            // GNOME strip on a dark card — no screen, no panel, no banner. `trayStrip` says so, the
            // same way `glyphs` and `desktop` do.
            FIXTURES[label]?.trayStrip
            ? renderTrayLight
            : null;
  if (!claim) return false;
  if (!document.getElementById("preview-css")) {
    document.head.append(
      el("style", { id: "preview-css" }, INDEX_CSS + GLYPH_CSS + DESKTOP_CSS + TRAY_LIGHT_CSS),
    );
  }
  claim(root, label);
  return true;
}
