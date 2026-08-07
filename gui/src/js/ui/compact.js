// The 360px compact panel (F6) — the quick check-in, and the whole of the tray surface.
// `02-shell.md` §"The 360px compact panel" and `10-tray.md`.
//
// Same information as the main screen, same order, about an eighth of the pixels. It is one
// component with two families: the `2a`/`4a` panel and the `10a` tray panel, which differ by a
// hexagon stroke, a few pixels of hero padding, and what sits at the bottom — two small buttons, or
// the menu section 10-tray.md specifies.
//
// MEASURED out of the eight in-scope dark frames (`2a Compact settled/syncing/needs you`,
// `4a Compact`, `10a Settled/Syncing/Offline/Paused`) plus the three `12a` light twins, node by node
// through `tools/fidelity/frames/*.json` — not transcribed from the prose, which gives ranges
// precisely where the frames disagree with each other. Four of those disagreements change what gets
// built:
//
//   · THE PANEL IS 362px WIDE, not 360. The prototype does not opt this node into `border-box` and
//     `base.css` opts the app in globally, so a panel written at its nominal 360 comes out 2px
//     narrower than every frame. DEVIATIONS.md §19 and §48 — F6 writes the drawn number.
//   · HERO PADDING IS A TABLE, NOT A RANGE. §"The 360px compact panel" gives
//     `22–30px 20–22px 14–24px`; the eight frames draw six distinct values, and the two families
//     differ from each other in the same state (settled is `26px 22px 20px` in the tray and
//     `28px 22px 22px` in the panel). Ranges cannot be interpolated; the table below is the source.
//   · THERE IS NO ATTENTION BAND IN ANY DRAWN COMPACT PANEL. Issue #170 and `02-shell.md` both
//     describe one (`margin:12px 14px`, ring dot, `›`). `2a Compact needs you` and `4a Compact` —
//     the two frames that would carry it — draw a full-width `Review them` button instead. Frame
//     wins under IMPLEMENTATION-PLAN §1.3 rule 2; DEVIATIONS.md §58.
//   · NO APPROVE ACTION LIVES HERE. `4a Compact` lists what is waiting and offers `Review them`,
//     and that is a safety property rather than a layout choice: approving a deletion from a panel
//     that fits in a corner of the screen is exactly the accident the deletion queue exists to
//     prevent. `deletions` therefore takes rows and one button; it takes no `onApprove`.
//
// The two primitives do the load-bearing work: `ui/hexagon.js` draws every mark and `ui/seam.js`
// owns the hairline (`compactPanel` / `trayPanel` in SEAM_SITES are this panel's two drawn sites).
// Neither is reimplemented here.

import { el } from "./el.js";
import { renderHexagon, updateHexagon, strokeForSize } from "./hexagon.js";
import { renderSeam, seamMask } from "./seam.js";
import { button } from "./controls.js";
import { transferRow, dot } from "./rows.js";
import { MAIN, TRAY } from "./copy.js";
// `data-fid` stamping (F8/F9). A no-op unless `?frame=` selected a fixture, so nothing reaches a
// user — it exists so the fidelity harness knows which drawn node each app node stands for.
import { fid } from "../fixtures/frames.js";

// ------------------------------------------------------------------------------ the states ----

/**
 * Six arrangements, not five. `02-shell.md` names five states — settled, syncing, needs-you, paused,
 * can't-reach-Proton — and `4a Compact` draws a sixth arrangement for the deletion queue: a smaller
 * mark, a 16px headline, two deletion rows and a `Review them` button. It is not any of the five at
 * different data, so it is its own key rather than a flag on `needsYou`.
 */
const STATES = ["settled", "syncing", "needsYou", "paused", "unreachable", "deletions"];

/**
 * Hero padding, per state and per family.
 *
 * Five of the six states are drawn in only one family, so the lookup falls back rather than throwing
 * — unlike `strokeForSize`, where a guess is a visibly wrong weight. Here the two families differ by
 * at most 2px where both are drawn, and `02-shell.md` requires paused and can't-reach in the panel
 * (they are only *drawn* in the tray), so a panel that refuses to render those two would be wrong
 * about the design rather than careful about it.
 */
const HERO_PAD = {
  settled: { panel: "28px 22px 22px", tray: "26px 22px 20px" },
  syncing: { panel: "22px 20px 16px", tray: "22px 20px 14px" },
  needsYou: { panel: "28px 22px 20px" },
  deletions: { panel: "24px 20px 18px" },
  paused: { tray: "26px 22px 20px" },
  unreachable: { tray: "26px 22px 20px" },
};

/** The headline's size and the gap above it. Both move with the state; neither is derivable. */
const HEADLINE = {
  settled: { size: "17px", top: "15px" },
  syncing: { size: "15px", top: "13px" },
  needsYou: { size: "17px", top: "15px" },
  paused: { size: "17px", top: "15px" },
  unreachable: { size: "17px", top: "15px" },
  deletions: { size: "16px", top: "14px" },
};

/**
 * The mark. 72px everywhere except the deletion queue, which draws 64 — a size `01-foundations.md`
 * §6 does not list at all (DEVIATIONS.md §20).
 *
 * The stroke is a FAMILY split and not a state one: every 72px mark in the `2a`/`12a` panels is 4.5
 * and every one in the `10a` tray panels is 4.6. `strokeForSize` already carries both, so it is
 * asked for the width rather than told one.
 *
 * `family: "tray"` is deliberately NOT passed to `renderHexagon`. That flag means the 16px tray
 * GLYPH — a longer dash (`70 230`), a 2.4s cycle and its own track colour, all so the motion reads
 * at icon size. The tray PANEL draws the ordinary in-window syncing mark at 72px, dash `62 238`,
 * 3.2s. Passing the family here would quietly redraw it as a scaled-up icon.
 */
function heroMark(state, family, count) {
  const size = state === "deletions" ? 64 : 72;
  const strokeWidth = strokeForSize(size, family === "tray" ? "tray" : "window");
  const common = { size, strokeWidth };

  switch (state) {
    case "settled":
      // No `masked`: `2a Compact settled` and `10a Settled` both draw `fill="none"`. The mask is a
      // property of sitting on the seam, and neither of them has one.
      return renderHexagon({ ...common, state: "settled" });
    case "syncing":
      // Masked, and this one is required rather than tidy: the seam runs the full height of the hero
      // and would otherwise show through the middle of the mark. DEVIATIONS.md §1.3 conflict 5.
      return renderHexagon({ ...common, state: "syncing", masked: true, numeral: count });
    case "needsYou":
    case "deletions":
      return renderHexagon({ ...common, state: "needsNumeral", tone: "decision", numeral: count });
    case "paused":
      return renderHexagon({ ...common, state: "paused" });
    case "unreachable":
      return renderHexagon({ ...common, state: "unreachable" });
    default:
      throw new Error(`compact: unknown state "${state}". Known: ${STATES.join(", ")}`);
  }
}

// ------------------------------------------------------------------------------ the pieces ----

/**
 * The seam labels — `THIS COMPUTER` left, `PROTON` right, 9.5px/600 at `.14em`.
 *
 * `Proton`, not `Proton Drive`: the panel is 360px wide and the full name is what the 1040 screen
 * uses. `copy.js` carries both, and the short one exists for exactly this site.
 *
 * Drawn only when the seam is (syncing). A settled panel has no two sides to name — nothing is
 * moving between them, which is `02-shell.md`'s own rule about when the seam appears.
 */
function seamLabels() {
  const local = fid(el("span", { class: "compact-label-local" }, MAIN.sideLocal), "labelLocal");
  const remote = fid(el("span", { class: "compact-label-remote" }, MAIN.sideRemoteCompact), "labelRemote");
  return fid(el("div", { class: "compact-labels" }, local, remote), "labels");
}

/**
 * The headline. On the syncing panel it wears the seam mask, because it is centred on the hairline.
 *
 * `position: false` — the mask's third part is already satisfied by the hero body wrapper, which is
 * `position: relative` for this reason. That is what 17 of the 34 drawn masks do, and it is what the
 * frame draws here: the headline carries a background and 14px of side padding, and nothing else.
 */
function headline(state, text) {
  const spec = HEADLINE[state];
  const node = el("div", { class: "compact-headline" }, text);
  node.style.fontSize = spec.size;
  node.style.marginTop = spec.top;
  if (state === "syncing") seamMask(node, { pad: 14, padY: 0, position: false });
  return fid(node, "headline");
}

/**
 * The sub-line. Two shapes, and they are not one shape with a font swap:
 *
 *   mono   `2 minutes ago`, `2 minutes ago · 12,480 files` — 11.5px `--text-5`, one line.
 *   prose  the paused and can't-reach sentences, and the needs-you pair — 12.5px `--text-4` at
 *          1.5 line-height, wrapping inside the panel.
 *
 * An array is rendered as lines separated by `<br>`, which is what `2a Compact needs you` draws:
 * two sentences that must break in a fixed place ("One file changed on both sides." /
 * "Two deletions are waiting.") rather than wherever the width happens to put them.
 */
function subLine(text, { mono = false } = {}) {
  const lines = Array.isArray(text) ? text : [text];
  const children = [];
  lines.forEach((line, i) => {
    if (i > 0) children.push(fid(el("br"), "subBreak", i - 1));
    children.push(line);
  });
  return fid(el("div", { class: "compact-sub" + (mono ? " is-mono" : "") }, children), "sub");
}

/**
 * The two small buttons at the bottom right of the panel: `Pause`/`Open`, or `Later` on the two
 * attention panels. `controls.js`'s `compact` rung is exactly this geometry — `5px 11px`, `--r-7`,
 * 11.5px — which is what that rung was measured from.
 */
function footerBar({ status = null, buttons = [] }) {
  const parts = [];
  if (status != null) parts.push(fid(el("span", { class: "compact-status" }, status), "footerStatus"));
  parts.push(fid(el("span", { class: "compact-spacer" }), "footerSpacer"));
  buttons.forEach((spec, i) => {
    parts.push(
      fid(
        button({
          kind: spec.kind ?? "quietOutlined",
          size: "compact",
          label: spec.label,
          onClick: spec.onClick,
        }),
        "footerButton",
        i,
      ),
    );
  });
  return fid(el("div", { class: "compact-footer" }, parts), "footer");
}

/**
 * The tray's menu section, in place of the two buttons (`10-tray.md` §"The panel").
 *
 * THE TWO SUB-LABELS ARE NOT DECORATION. `Close window · keeps syncing` and `Quit · stops syncing`
 * are, in 10-tray.md's words, the single worst misunderstanding a tray app can cause; the v1 build
 * spelled them out and this one keeps them. They are baseline-aligned at `gap:8px`, which is why a
 * row with a sub-label is a flex row and a row without one is not — the frames draw exactly that
 * difference, and it is also why `has-sub` is a class rather than a `<span>` that is sometimes empty.
 *
 * The first row carries `--panel-raised`, as every drawn panel does: it is the default action, and
 * hovering any row gives it the same fill.
 */
function menuSection(rows) {
  const children = [];
  // ONE index across rows AND separators, because that is what the frames' node keys count: a
  // separator is a `<div>` like every other child, so the row after the rule is `div[4]`, not
  // `div[3]`. Counting rows separately would map every post-separator row onto its neighbour.
  rows.forEach((row, at) => {
    if (row.separator) {
      children.push(fid(el("div", { class: "compact-menu-sep" }), "menuSep", at));
      return;
    }
    // A row without a sub-label holds a bare text node, exactly as the frames draw it. Wrapping it
    // in a `<span>` for symmetry would put a node where the ground truth has none.
    const label = row.sub
      ? fid(el("span", { class: "compact-menu-label" }, row.label), "menuLabel", at)
      : row.label;
    const sub = row.sub ? fid(el("span", { class: "compact-menu-sub" }, row.sub), "menuSub", at) : null;
    children.push(
      fid(
        el(
          "button",
          {
            class: "compact-menu-row" + (row.sub ? " has-sub" : "") + (at === 0 ? " is-first" : ""),
            type: "button",
            onClick: row.onClick ?? null,
          },
          label,
          sub,
        ),
        "menuRow",
        at,
      ),
    );
  });
  return fid(el("div", { class: "compact-menu" }, children), "menu");
}

/**
 * A deletion waiting in the queue, at panel scale. `4a Compact` draws two.
 *
 * NOT `rows.js`'s `deletionCard` at a smaller size — that card is five times the height and carries
 * a facts strip, a typed-`DELETE` gate and two buttons, none of which appear here. A shared builder
 * would have to grow a second layout, which is why `rows.js` says so in a comment and leaves this
 * one to F6.
 *
 * The dot follows the design's crimson grammar and `rows.js` already encodes it: a permanent
 * deletion is a FILL (it will happen, and it cannot be undone), a recoverable one is an OUTLINE
 * (it is waiting on you). Severity also picks the tint, one notch softer than the 1040 screen's —
 * DEVIATIONS.md §52a.
 */
function deletionRow({ severity, name, note }, i) {
  if (severity !== "permanent" && severity !== "recoverable") {
    throw new Error(`compact: deletion severity must be "permanent" or "recoverable", got "${severity}"`);
  }
  const severityDot = fid(
    dot({ tone: severity === "permanent" ? "destructive" : "decision", size: 6 }),
    "deletionDot",
    i,
  );
  const head = fid(
    el(
      "div",
      { class: "compact-deletion-head" },
      severityDot,
      fid(el("span", { class: "compact-deletion-name" }, name), "deletionName", i),
    ),
    "deletionHead",
    i,
  );
  return fid(
    el(
      "div",
      { class: `compact-deletion compact-deletion-${severity}` },
      head,
      fid(el("div", { class: "compact-deletion-note" }, note), "deletionNote", i),
    ),
    "deletionRow",
    i,
  );
}

// -------------------------------------------------------------------------------- the panel ----

/**
 * Build the panel.
 *
 * `state` picks the arrangement; `family` picks the panel (`2a`/`4a`) or the tray (`10a`) form.
 * Everything else is content — the component computes no strings and reads no store, because the
 * same panel is mounted by the main screen (S1), by the tray window (S8) and by the fidelity
 * fixtures, and each of those knows something different about where its data comes from.
 *
 * Returns the panel element. It is NOT wrapped: `10a In situ` sits it directly on a desktop mock and
 * S8's tray window has nothing else in it.
 */
export function renderCompactPanel(opts = {}) {
  const {
    state = "settled",
    family = "panel",
    headline: headlineText,
    sub = null,
    subMono = false,
    meta = null,
    count = null,
    transfers = [],
    deletions = [],
    action = null,
    footer = null,
    menu = null,
  } = opts;

  if (!STATES.includes(state)) {
    throw new Error(`compact: unknown state "${state}". Known: ${STATES.join(", ")}`);
  }
  if (family !== "panel" && family !== "tray") {
    throw new Error(`compact: family must be "panel" or "tray", got "${family}"`);
  }
  if (headlineText == null) throw new Error("compact: a headline is required — every drawn panel has one");
  // Two pairs where passing both means the caller believes something this panel cannot do. Each has
  // a silent form — one list wins and the other vanishes — and a panel that quietly drops the
  // deletions it was handed is the worst thing in this component to get wrong.
  if (transfers.length && deletions.length) {
    throw new Error("compact: a panel shows transfers or deletions, never both — no frame draws that");
  }
  if (menu && footer) {
    throw new Error("compact: the tray menu REPLACES the two footer buttons (10-tray.md) — pass one");
  }

  const pad = HERO_PAD[state];
  const seamed = state === "syncing";

  // --- the hero. Two shapes: a centred column, or — when the seam is drawn — a block holding the
  // seam, the two side labels and a centred column of its own. The seam is absolutely positioned
  // against the hero, so the hero is what has to be `position: relative`, and the body wrapper is
  // what lets the headline mask without positioning itself (seam.js rule 3).
  const mark = fid(heroMark(state, family, count), "hexagon");
  mark.querySelectorAll("path").forEach((path, i) => fid(path, "hexPath", i));
  mark.querySelectorAll("rect").forEach((rect, i) => fid(rect, "hexRect", i));
  fid(mark.querySelector("text"), "hexNumeral");

  const title = headline(state, headlineText);
  const subNode = sub == null ? null : subLine(sub, { mono: subMono });
  const metaNode = meta == null ? null : fid(el("div", { class: "compact-meta" }, meta), "meta");
  const actionNode =
    action && state !== "deletions"
      ? fid(
          button({
            kind: action.kind ?? "decision",
            label: action.label,
            onClick: action.onClick,
            padding: "10px",
            radius: "var(--r-9)",
            fontSize: "13px",
            class: "compact-action-btn",
          }),
          "action",
        )
      : null;

  const hero = seamed
    ? fid(
        el(
          "div",
          { class: "compact-hero is-seamed" },
          // First child, and nothing wraps it: DOM order is half of the mask rule — siblings that
          // come later and are positioned paint over the line, which is how the headline hides it.
          fid(renderSeam({ site: family === "tray" ? "trayPanel" : "compactPanel" }), "seam"),
          seamLabels(),
          // The sub-line, the meta line and the action go INSIDE the body, not beside it. No syncing
          // frame draws any of the three, so this path is unmeasured — but the alternative is a
          // caller passing `sub` to a syncing panel and watching it vanish, and an unmeasured node
          // is something the fidelity gate catches (it moves the panel's own height) while a
          // silently dropped one is not.
          fid(
            el("div", { class: "compact-hero-body" }, mark, title, subNode, metaNode, actionNode),
            "heroBody",
          ),
        ),
        "hero",
      )
    : fid(el("div", { class: "compact-hero is-column" }, mark, title, subNode, metaNode, actionNode), "hero");
  hero.style.padding = pad[family] ?? pad.panel ?? pad.tray;

  // --- the blocks below it. `2a Compact syncing` and `4a Compact` share one container geometry —
  // `padding:0 14px 12px`, `gap:6px` — for two quite different lists, which is why the class is
  // named for the slot rather than for what goes in it.
  const rows = transfers.length
    ? fid(
        el(
          "div",
          { class: "compact-list" },
          transfers.map((t, i) => {
            const row = fid(transferRow({ ...t, size: "compact" }), "transferRow", i);
            fid(row.querySelector(".transfer-name"), "transferName", i);
            fid(row.querySelector(".transfer-arrow"), "transferArrow", i);
            fid(row.querySelector(".transfer-track"), "transferTrack", i);
            fid(row.querySelector(".transfer-fill"), "transferFill", i);
            return row;
          }),
        ),
        "transfers",
      )
    : deletions.length
      ? fid(el("div", { class: "compact-list" }, deletions.map(deletionRow)), "deletions")
      : null;

  // The deletion queue's `Review them` is a block of its own beneath the rows rather than a child of
  // the hero — it is answering the list, not the headline.
  const actionBlock =
    action && state === "deletions"
      ? fid(
          el(
            "div",
            { class: "compact-action" },
            fid(
              button({
                kind: action.kind ?? "primarySoft",
                label: action.label,
                onClick: action.onClick,
                padding: "10px",
                radius: "var(--r-9)",
                fontSize: "12.5px",
                class: "compact-action-btn",
              }),
              "actionButton",
            ),
          ),
          "actionBlock",
        )
      : null;

  const tail = menu ? menuSection(menu) : footer ? footerBar(footer) : null;

  const panel = el(
    "div",
    {
      class:
        "compact-panel" +
        // `is-tray` styles nothing today, and is here rather than in S8 because it is the panel's
        // own class to give. 10-tray.md asks the tray form for `border:1px solid rgba(255,255,255,.1)`
        // — it floats over the desktop, not over the app surface — and all four drawn `10a` panels
        // use `#23262D` like every other compact panel, so the frame wins and there is nothing to
        // apply (DEVIATIONS.md §58d). S8 owns a borderless always-on-top window and will want the
        // hook; giving it now means the tray form is never selected by a `[data-state]` guess.
        (family === "tray" ? " is-tray" : "") +
        // The panel's own edge goes crimson when something is waiting on you — measured on
        // `2a Compact needs you`, `4a Compact` and `12a Compact needs light`, all three at .3 alpha.
        (state === "needsYou" || state === "deletions" ? " is-attention" : ""),
      "data-state": state,
    },
    hero,
    rows,
    actionBlock,
    tail,
  );
  return fid(panel, "root");
}

/**
 * Patch a rendered panel across a status poll, without rebuilding it.
 *
 * The same constraint `updateHexagon` and `updateHeader` were written for, and it bites hardest
 * here: the syncing panel holds two animated hexagon segments and two progress bars, and the tray
 * polls on the same ~2s cadence as the window. A rebuild restarts both animations from 0% and drops
 * keyboard focus out of the menu — and the tray panel is a surface people click through quickly.
 *
 * Returns false when the panel's SHAPE changed — a different state, a different number of rows, a
 * line that was not there before — which is the caller's signal to render a fresh one. Same contract
 * as `updateHeader`.
 *
 * A SLOT THAT IS ASKED FOR AND IS NOT THERE IS A SHAPE CHANGE, never a no-op. The tempting version
 * writes what it can find and returns true, and its failure is silent: the panel goes on showing
 * `Nothing is lost. 4 changes are waiting…` with no line saying when it will next try, because the
 * meta line did not exist when it was built and nothing said so.
 *
 * VERIFIED BY DRIVING IT, not by a unit test — `gui/test` has no DOM (no jsdom, deliberately: the
 * fidelity gate is this frontend's real check) and this function is all DOM. Run against the
 * `2a Compact syncing` preview in headless Chromium: the headline, the numeral, both progress fills
 * and the footer status all patch; `querySelector("svg")` returns the SAME node afterwards, which is
 * the property that keeps the two `hexup`/`hexdn` animations running; and a wrong state, a changed
 * row count, an absent meta line and an array `sub` each return false with the panel's text
 * unchanged. It has no consumer until S1 and S8, which is exactly why it was driven now.
 */
export function updateCompactPanel(node, opts = {}) {
  if (!node) return false;
  const { state, headline: headlineText, sub, meta, count, transfers, footer } = opts;
  if (state && node.dataset.state !== state) return false;

  const rows = node.querySelectorAll(".transfer-row");
  if (transfers && transfers.length !== rows.length) return false;

  // Every slot is resolved BEFORE anything is written, so a `false` return leaves the panel exactly
  // as it was rather than half-patched. The caller discards it either way; a function that mutates
  // and then reports failure is one someone eventually trusts the wrong way round.
  const writes = [];
  const need = (selector, text) => {
    const target = node.querySelector(selector);
    if (target) writes.push([target, text]);
    return Boolean(target);
  };

  if (headlineText != null && !need(".compact-headline", headlineText)) return false;
  // Only the single-line form is patched. A multi-line sub is a `<br>` between text nodes, and
  // rewriting it through textContent would collapse the break the design put there on purpose —
  // so a changed line count reports a shape change instead.
  if (sub != null) {
    if (Array.isArray(sub) || !need(".compact-sub", sub)) return false;
    if (node.querySelector(".compact-sub br")) return false;
  }
  if (meta != null && !need(".compact-meta", meta)) return false;
  if (footer?.status != null && !need(".compact-status", footer.status)) return false;

  for (const [target, text] of writes) {
    if (target.textContent !== text) target.textContent = text;
  }
  if (count !== undefined) updateHexagon(node.querySelector(".compact-hero svg"), { numeral: count });
  if (transfers) {
    transfers.forEach((t, i) => {
      const fill = rows[i]?.querySelector(".transfer-fill");
      if (fill) fill.style.width = `${Math.max(0, Math.min(1, t.progress ?? 0)) * 100}%`;
    });
  }
  return true;
}

// --------------------------------------------------------------------------- the tray menu ----

/**
 * The menu rows, per state — `10-tray.md` §"Menu contents by state", as a table rather than as five
 * hand-built lists at the call site.
 *
 * `id` is what the caller dispatches on; S8 owns wiring them to the daemon, and the tray already has
 * every one of these actions in its v1 text menu. The labels come from `copy.js` so the panel, the
 * native right-click menu and the window cannot drift apart — three surfaces quote `Sync now`.
 *
 * Read the table alongside 10-tray.md and two things stand out, both deliberate. `Sync now` is
 * ABSENT while syncing, because it would do nothing. And the two states that are not moving files —
 * paused and can't-reach — lead with the row that fixes them and drop `Close window` entirely: with
 * nothing syncing, "keeps syncing" would be a lie.
 */
export const TRAY_MENU = {
  settled: [
    { id: "open", label: TRAY.open },
    { id: "syncNow", label: TRAY.syncNow },
    { id: "pause", label: TRAY.pause },
    { separator: true },
    { id: "closeWindow", label: TRAY.closeWindow, sub: TRAY.closeWindowSub },
    { id: "quit", label: TRAY.quit, sub: TRAY.quitSub },
  ],
  syncing: [
    { id: "open", label: TRAY.open },
    { id: "pause", label: TRAY.pause },
    { separator: true },
    { id: "closeWindow", label: TRAY.closeWindow, sub: TRAY.closeWindowSub },
    { id: "quit", label: TRAY.quit, sub: TRAY.quitSub },
  ],
  // The panel itself carries `Review them`, so the menu does not repeat it.
  needsYou: [
    { id: "open", label: TRAY.open },
    { id: "syncNow", label: TRAY.syncNow },
    { id: "pause", label: TRAY.pause },
    { separator: true },
    { id: "closeWindow", label: TRAY.closeWindow, sub: TRAY.closeWindowSub },
    { id: "quit", label: TRAY.quit, sub: TRAY.quitSub },
  ],
  paused: [
    { id: "resume", label: TRAY.resume },
    { id: "open", label: TRAY.open },
    { separator: true },
    { id: "quit", label: TRAY.quit, sub: TRAY.quitSub },
  ],
  unreachable: [
    { id: "tryAgain", label: TRAY.tryAgain },
    { id: "open", label: TRAY.open },
    { separator: true },
    { id: "quit", label: TRAY.quit, sub: TRAY.quitSub },
  ],
};

/**
 * The menu for a state, with one handler bound to every row.
 *
 * `onSelect(id)` rather than a handler per row: the five lists share nine actions between them, and
 * a caller that has to supply five objects of callbacks will get one of them wrong on the state it
 * tests least — which, for a tray, is `unreachable`.
 */
export function trayMenu(state, onSelect = null) {
  const rows = TRAY_MENU[state];
  if (!rows) {
    throw new Error(
      `compact: no tray menu for state "${state}". Known: ${Object.keys(TRAY_MENU).join(", ")}`,
    );
  }
  return rows.map((row) =>
    row.separator ? row : { ...row, onClick: onSelect ? () => onSelect(row.id) : null },
  );
}
