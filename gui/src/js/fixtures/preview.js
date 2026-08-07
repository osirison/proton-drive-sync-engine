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

import { FIXTURES } from "./frames.js";
import { el } from "../ui/el.js";

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

/** `?frame=<label>` for this frame, plus the theme the light set wants. See `previewTheme`. */
function href(label) {
  const query = new URLSearchParams({ frame: label });
  if (setOf(label) === "12a") query.set("theme", "light");
  return `?${query}`;
}

/**
 * The index. Rendered only for `?frames`, and it takes over the window — the shell never boots, so
 * nothing polls a daemon behind it.
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
  const claim = query.has("frames") ? renderIndex : label != null && !FIXTURES[label] ? renderUnknown : null;
  if (!claim) return false;
  if (!document.getElementById("preview-css")) {
    document.head.append(el("style", { id: "preview-css" }, INDEX_CSS));
  }
  claim(root, label);
  return true;
}
