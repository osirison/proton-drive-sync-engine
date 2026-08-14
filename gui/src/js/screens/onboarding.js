// The onboarding takeover (S7) — two steps and three surfaces around them. `09-onboarding.md`.
//
// Two steps live in the takeover body (`9a Folders`, `9a Review`); `9a CLI missing`, `9a First sync`
// and `9a Consent` are dialogs, and app.js drives which one is open. The takeover has no footer nav,
// so both steps carry an action bar with the window's own 18px bottom margin.
//
// Phase 1 omissions, each in DEVIATIONS §79 with its issue: every per-side file count and byte total
// (#240), the account line (#241), `Needs 38.4 GB free` (#206 — C4 answers the other half), the
// already-matching count (#242), the ETA (#229), the split progress bar (#243), the merged totals
// (#207), the install command box (#218), and four buttons with no destination — three of them
// #244 (`Add skip rules`, `See all N actions`, `Installation help`, the last also #218) and
// `Browse Proton Drive…`, which is #99.

import { el } from "../ui/el.js";
import { MAIN, ONBOARDING, PLAN } from "../ui/copy.js";
import { count, since } from "../ui/format.js";
import { renderHexagon, updateHexagon } from "../ui/hexagon.js";
import { renderSeam, seamMask } from "../ui/seam.js";
import { button, textInput, checkbox, setButtonKind } from "../ui/controls.js";
import { consentPanel, warnGlyph } from "../ui/bands.js";
import { renderActionBar } from "../ui/chrome.js";
import { dot, eyebrow } from "../ui/rows.js";
import { fid } from "../fixtures/frames.js";

/** The marks, at the five sizes this flow draws them. 80 is two-valued in `strokeForSize`. */
const REVIEW_MARK = 80;
const REVIEW_MARK_STROKE = 4.6;
const FACT_MARK = 13;
const MERGE_MARK = 116;
const CONSENT_MARK = 76;
const CLI_MARK = 34;

// ------------------------------------------------------------------------------- the model ----

/**
 * What the flow is showing. `checking` outranks a payload for the same reason `5a Checking` does:
 * the plan on screen is the old one.
 */
export function bodyOf({ step = "folders", dryRun = null, checking = false, error = null } = {}) {
  if (step !== "review") return "folders";
  if (checking) return "checking";
  if (error) return "failed";
  return dryRun ? "review" : "checking";
}

/**
 * `See all 471 actions` against a 474-row plan: `SkipUnsupported` IS a plan row, so `total` counts
 * it and the button names what will actually happen. Reading `total` straight draws 474.
 */
export function actionsThatHappen(summary) {
  if (!summary) return null;
  return Math.max(0, (summary.total ?? 0) - (summary.skipped_unsupported ?? 0));
}

/**
 * Has the merge finished, and did it work?
 *
 * `waiting` · `failed` · `done`, from one status reply plus what the flow remembers. Pure, because
 * every arm of it is a claim about someone's files: `done` opens a dialog that says `Both sides now
 * match` and `Nothing was deleted`.
 *
 * A COMPLETED PASS IS NOT A SUCCESSFUL ONE. `reconcile_blocking` bumps `reconcile_seq` either way —
 * "the attempt is complete (recorded either way)", src/daemon.rs — and records the reason in
 * `last_error`, so the counter alone would call a failed first sync a finished merge.
 *
 * @param reply    the daemon's `response`, or null when it has not answered
 * @param mergeSeq `reconcile_seq` when the merge was started, or null if it had not answered then
 */
export function mergeOutcomeOf(reply, mergeSeq = null) {
  if (!reply || reply.syncing) return "waiting";
  const seq = reply.reconcile_seq ?? 0;
  // THE COUNTER, NOT `last_sync_epoch_secs`, on both arms. `self.last_sync` is set inside the Ok
  // path of `reconcile_blocking_inner` (src/daemon.rs) — a pass that FAILS never sets it — while the
  // counter advances either way. Testing the timestamp left a failed first sync on a machine that
  // had never synced reporting `waiting` forever, with the merge dialog claiming progress and no way
  // out of it.
  const completed = mergeSeq != null ? seq > mergeSeq : seq > 0 || Boolean(reply.last_sync_epoch_secs);
  if (!completed) return "waiting";
  return reply.last_error ? "failed" : "done";
}

/** Can step 1 proceed? Both roots must be non-empty — the daemon has no folder pair otherwise. */
export function pairReady({ local = "", remote = "" } = {}) {
  return Boolean(String(local).trim() && String(remote).trim());
}

// -------------------------------------------------------------------- step 1 · which folders ----

/** One side of the pair. The two are not mirrors: only the local side has a picker. */
function folderSide(props, s) {
  const local = s === 0;
  const side = fid(el("div", { class: `ob-side${local ? "" : " is-remote"}` }), "side", s);

  const label = fid(el("div", { class: "ob-side-label" }), "sideLabel", s);
  const mark = fid(dot({ tone: local ? "up" : "down", size: 8 }), "sideDot", s);
  // A `<span>`, not `eyebrow()`'s `<div>`: the frame draws both label rows' text inline, and
  // `display` is asserted. The class carries the type; the tag carries the box.
  const label2 = fid(
    el(
      "span",
      { class: `eyebrow eyebrow-${local ? "up" : "down"} eyebrow-start` },
      local ? MAIN.sideLocal : MAIN.sideRemote,
    ),
    "sideEyebrow",
    s,
  );
  label.append(...(local ? [mark, label2] : [label2, mark]));

  const card = fid(el("div", { class: "ob-card" }), "card", s);
  // The remote path is EDITABLE and the local one is not: `list_remote` reads a path and no picker
  // exists for one (S6 settled the same asymmetry on `8a Settings`). #99.
  // UNMAPPED on the remote side, deliberately: an `<input>` is `inline-block` with `overflow:clip`
  // by UA rule and the frame draws a `<div>`, so the two can never agree on either — a construction
  // difference, which is not what `known-deviations.mjs` is for. §79. The fixture's `cardPath`
  // returns `null` at `s === 1` to say so, which is what keeps the unstamped gate off it (#248).
  const path = local
    ? fid(el("div", { class: "ob-card-path" }, props.local || ""), "cardPath", s)
    : textInput({
        value: props.remote ?? "",
        mono: true,
        class: "input is-mono ob-card-input",
        "aria-label": MAIN.sideRemote,
        "data-field": "remote_root",
        onInput: (e) => props.handlers?.onRoot?.("remote", e.target.value),
      });
  card.append(path);
  // The stats row and the account line are omitted, not blanked: nothing counts the files or bytes
  // under a candidate folder on either side (#240), and no command sees an account (#241).
  if (local) {
    card.append(
      fid(
        button({
          kind: "secondaryOutlined",
          size: "standard",
          label: ONBOARDING.chooseLocal,
          padding: "10px",
          class: "ob-card-button",
          onClick: () => props.handlers?.onChooseLocal?.(),
        }),
        "cardButton",
        s,
      ),
    );
  }
  side.append(label, card);
  // `Browse Proton Drive…` is the remote side's button and has nowhere to go, so the side's helper
  // is the only thing under its card.
  if (local) {
    side.append(fid(el("div", { class: "ob-side-note" }, ONBOARDING.emptyIsFine), "sideNote", s));
  }
  return side;
}

function foldersBody(props) {
  const title = fid(
    el(
      "div",
      { class: "ob-title-block" },
      fid(el("div", { class: "ob-title" }, ONBOARDING.foldersTitle), "title"),
      fid(el("div", { class: "ob-sub" }, ONBOARDING.foldersSub), "sub"),
    ),
    "titleBlock",
  );

  const block = fid(el("div", { class: "ob-folders" }), "foldersBlock");
  block.append(fid(renderSeam({ site: "onboardingFolders" }), "seam"));
  const grid = fid(el("div", { class: "ob-grid" }), "grid");
  grid.append(folderSide(props, 0), folderSide(props, 1));
  block.append(grid);

  // The skip prompt keeps its sentence and loses its button: there is no skip-rules editor inside a
  // takeover that cannot be dismissed, and leaving for Settings is a one-way door on a machine with
  // no daemon (#244). The sentence's "or any time later in Settings" is the half that works today.
  const skip = fid(el("div", { class: "ob-skip" }), "skipPanel");
  skip.append(
    fid(warnGlyph(), "skipGlyph"),
    fid(el("div", { class: "ob-skip-text" }, ONBOARDING.skipHint), "skipText"),
  );
  block.append(skip);
  return [title, block];
}

// ----------------------------------------------------------------- step 2 · nothing deleted ----

/** One of the two counts. `size` is #191/#206 — the unit line drops its byte clause. */
function countSide(props, s, summary) {
  const up = s === 0;
  const side = fid(el("div", { class: `ob-count-side${up ? "" : " is-remote"}` }), "countSide", s);
  side.append(
    fid(
      eyebrow({
        tone: up ? "up" : "down",
        text: up ? ONBOARDING.goingUp : ONBOARDING.comingDown,
        align: up ? "start" : "end",
      }),
      "countEyebrow",
      s,
    ),
  );
  const files = (up ? summary?.uploads : summary?.downloads) ?? null;
  side.append(
    fid(
      el(
        "div",
        { class: "ob-count-row" },
        fid(el("span", { class: "ob-numeral" }, count(files)), "countNumeral", s),
        fid(el("span", { class: "ob-count-unit" }, ONBOARDING.sideUnit(files, null)), "countUnit", s),
      ),
      "countRow",
      s,
    ),
  );
  // Up: the sentence. Down: the free-space line, with only the half C4 answers — `Needs 38.4 GB
  // free` needs a byte total no level of the dry-run surface carries (#206).
  const note = up
    ? ONBOARDING.goingUpSub
    : props.freeSpace?.available != null
      ? ONBOARDING.freeSpaceHave(props.freeSpace.available)
      : null;
  if (note) side.append(fid(el("div", { class: "ob-count-note" }, note), "countNote", s));
  return side;
}

/**
 * The four fact rows, in the frame's order, minus the ones with no source.
 *
 * The last row is the point of the screen — zero destructive actions as a positive fact — so it is
 * drawn only when the plan really has none. A plan that would delete something cannot say this, and
 * no frame draws that state.
 */
export function factRows(summary) {
  const rows = [];
  // `11,798 files already match on both sides` counts files the plan does NOT act on; `PlanSummary`
  // has no such field, by construction rather than omission. #242.
  if (summary?.conflicts) {
    rows.push({
      at: 1,
      tone: "decision",
      label: ONBOARDING.differ(summary.conflicts),
      note: ONBOARDING.differSub,
      noteTone: "decision",
    });
  }
  if (summary?.skipped_unsupported) {
    rows.push({
      at: 2,
      tone: "inert",
      // The drawn sentence names the kinds (`a socket and two shortcuts`) and nothing enumerates
      // them — the files never enter the index (#232). The count is the half that is true.
      label: ONBOARDING.cannotSyncPlain(summary.skipped_unsupported),
      note: ONBOARDING.skipped,
      dim: true,
    });
  }
  if (summary && !summary.destructive_actions) {
    rows.push({ at: 3, tone: "hexagon", label: ONBOARDING.nothingDeleted, note: ONBOARDING.eitherSide });
  }
  return rows;
}

function factsBlock(summary) {
  const rows = factRows(summary);
  if (!rows.length) return null;
  const block = fid(el("div", { class: "ob-facts" }), "facts");
  // KEYED BY THE ROW IT STANDS FOR, not by its position here: row 0 is omitted (#242), so the app's
  // first row is the frame's second and an app-order index compares a ringed dot against a filled
  // one. `at` is the drawn index; `i` is only "is this the last one drawn".
  for (const [i, row] of rows.entries()) {
    const at = row.at;
    const node = fid(el("div", { class: "ob-fact" + (i === rows.length - 1 ? " is-last" : "") }), "fact", at);
    if (row.tone === "hexagon") {
      const mark = fid(
        renderHexagon({ size: FACT_MARK, state: "outline", flexNone: true, class: "ob-fact-mark" }),
        "factMark",
        at,
      );
      fid(mark.querySelector("path"), "factMarkPath", at);
      node.append(mark);
    } else {
      node.append(fid(dot({ tone: row.tone, size: 6 }), "factDot", at));
    }
    node.append(
      fid(el("span", { class: "ob-fact-label" + (row.dim ? " is-dim" : "") }, row.label), "factLabel", at),
      fid(
        el("span", { class: `ob-fact-note${row.noteTone ? ` tone-${row.noteTone}` : ""}` }, row.note),
        "factNote",
        at,
      ),
    );
    block.append(node);
  }
  return block;
}

function reviewBody(props) {
  const summary = props.dryRun?.report?.summary ?? null;

  const hero = fid(el("div", { class: "ob-hero" }), "hero");
  hero.append(fid(renderSeam({ site: "onboardingReview" }), "heroSeam"));
  const mark = fid(
    renderHexagon({
      size: REVIEW_MARK,
      state: "settled",
      strokeWidth: REVIEW_MARK_STROKE,
      masked: true,
      class: "ob-hero-mark",
    }),
    "heroMark",
  );
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "heroMarkPath", i);
  const title = fid(el("div", { class: "ob-hero-title" }, ONBOARDING.reviewTitle), "heroTitle");
  const sub = fid(el("div", { class: "ob-hero-sub" }, ONBOARDING.reviewSub), "heroSub");
  seamMask(title, { pad: 18 });
  seamMask(sub, { pad: 18, padY: 2 });
  hero.append(mark, title, sub);

  const body = fid(el("div", { class: "ob-review" }), "body");
  const counts = fid(el("div", { class: "ob-counts" }), "counts");
  counts.append(countSide(props, 0, summary), countSide(props, 1, summary));
  body.append(counts);
  const facts = factsBlock(summary);
  if (facts) body.append(facts);
  // `See all N actions` has nowhere to open — the plan screen is a door the takeover covers (#244) —
  // so the row is its timing line alone. `about 25 minutes to finish` is #229.
  if (props.checkedAt != null) {
    body.append(
      fid(
        el(
          "div",
          { class: "ob-timing" },
          fid(
            el("span", { class: "ob-timing-text" }, ONBOARDING.workedOutPlain(since(props.checkedAt))),
            "timingText",
          ),
        ),
        "timing",
      ),
    );
  }
  return [hero, body];
}

/**
 * The rehearsal in flight, and the rehearsal that failed. Neither is drawn: `5a Checking` and S4's
 * failed body are the shapes, borrowed with their copy, because a takeover with a blank middle is
 * where a machine with no `proton-syncd` on its PATH would otherwise land.
 */
function checkingBody() {
  const body = fid(el("div", { class: "ob-working" }), "working");
  const mark = renderHexagon({ size: MERGE_MARK, state: "syncing", dryRun: true, class: "ob-working-mark" });
  body.append(
    mark,
    el("div", { class: "ob-working-title" }, PLAN.checkingTitle),
    el("div", { class: "ob-working-sub" }, PLAN.checkingSub),
  );
  return [body];
}

function failedBody(error) {
  const body = fid(el("div", { class: "ob-working" }), "working");
  body.append(
    renderHexagon({ size: REVIEW_MARK, state: "warning", tone: "decision", strokeWidth: REVIEW_MARK_STROKE }),
    el("div", { class: "ob-working-title" }, PLAN.failedTitle),
    el("div", { class: "ob-working-sub" }, PLAN.failedSub),
    el("div", { class: "ob-working-error mono" }, error),
  );
  return [body];
}

// -------------------------------------------------------------------------------- the screen ----

/** What the last render was built from, so a poll can decide whether to build at all. */
let view = null;

/**
 * Everything the body draws, and nothing else.
 *
 * The two typed roots are absent on purpose: step 1 holds a live `<input>`, and rebuilding it on the
 * ~2s poll would move the caret to the end of whatever someone was typing. Step 2 has no field, so
 * its relative time is free to be in here.
 */
function signatureOf(props) {
  const body = bodyOf(props);
  // The LOCAL path is in here and the remote one is not, and the asymmetry is the point: the local
  // path is text the picker replaces (so it must rebuild) and the remote one is the field's own
  // value (so it must not).
  if (body === "folders") return JSON.stringify(["folders", props.local]);
  if (body === "checking") return "checking";
  if (body === "failed") return JSON.stringify(["failed", props.error]);
  const s = props.dryRun?.report?.summary ?? null;
  return JSON.stringify([
    "review",
    s && [s.uploads, s.downloads, s.conflicts, s.skipped_unsupported, s.destructive_actions],
    props.freeSpace?.available ?? null,
    props.checkedAt == null ? null : since(props.checkedAt),
  ]);
}

/** The takeover body, as window-root siblings — the seam's `left:50%` resolves against the window. */
export function renderOnboarding(props = {}) {
  const nodes = (() => {
    switch (bodyOf(props)) {
      case "checking":
        return checkingBody();
      case "failed":
        return failedBody(props.error);
      case "review":
        return reviewBody(props);
      default:
        return foldersBody(props);
    }
  })();
  view = { sig: signatureOf(props), nodes };
  return nodes;
}

/** The poll's path: rebuild only when something the body draws has moved, else `null`. */
export function updateOnboarding(props = {}) {
  if (!view) return null;
  if (signatureOf(props) === view.sig) return null;
  return renderOnboarding(props);
}

/** Drop the cached view — the next mount builds from scratch. */
export function unmountOnboarding() {
  view = null;
}

/** What a rebuild of the footer bar would change, as opposed to what leaving it alone can carry. */
export function onboardingBarShape(props = {}) {
  return `${bodyOf(props)}|${pairReady(props)}`;
}

/** The footer action bar. Per-STATE, like the plan screen's: the two steps do not share one. */
export function renderOnboardingBar(props = {}) {
  const body = bodyOf(props);
  const handlers = props.handlers ?? {};
  if (body === "folders") {
    const ready = pairReady(props);
    const next = fid(
      button({
        kind: "primary",
        size: "bar",
        label: ONBOARDING.seeWhatHappens,
        padding: "11px 24px",
        onClick: () => handlers.onNext?.(),
      }),
      "barPrimary",
    );
    // Built live and then repainted: `button()` attaches no listener when the kind is a disabled
    // one, so a button born `primaryDisabled` and later armed paints live and does nothing.
    if (!ready) setButtonKind(next, "primaryDisabled");
    const bar = renderActionBar({
      consequence: fid(
        el("span", { class: "bar-consequence tone-quiet" }, ONBOARDING.nothingUntilApproved),
        "barText",
      ),
      primary: next,
      bottom: 18,
    });
    fid(bar.querySelector(".shell-spacer"), "barSpacer");
    return fid(bar, "bar");
  }
  if (body !== "review") {
    // Checking and failed: `Back` is the only way out of a rehearsal that has not answered, and the
    // failed body's `Check again` is the loud one because there is no plan to run.
    const bar = renderActionBar({
      consequence: fid(back(handlers), "barBack"),
      primary:
        body === "failed"
          ? fid(
              button({
                kind: "primary",
                size: "bar",
                label: PLAN.checkAgain,
                padding: "11px 22px",
                onClick: () => handlers.onCheck?.(),
              }),
              "barPrimary",
            )
          : null,
      bottom: 18,
    });
    fid(bar.querySelector(".shell-spacer"), "barSpacer");
    return fid(bar, "bar");
  }
  const bar = renderActionBar({
    consequence: fid(back(handlers), "barBack"),
    primary: fid(
      button({
        kind: "primary",
        size: "bar",
        label: ONBOARDING.start,
        padding: "11px 24px",
        onClick: () => handlers.onStart?.(),
      }),
      "barPrimary",
    ),
    bottom: 18,
  });
  fid(bar.querySelector(".shell-spacer"), "barSpacer");
  return fid(bar, "bar");
}

/** Every bar path goes through here so `dataset.shape` cannot be set on one and missed on another. */
export function renderOnboardingFooter(props = {}) {
  const bar = renderOnboardingBar(props);
  bar.dataset.shape = onboardingBarShape(props);
  return bar;
}

function back(handlers) {
  return button({
    kind: "quietOutlined",
    size: "bar",
    label: ONBOARDING.back,
    padding: "11px 18px",
    onClick: () => handlers.onBack?.(),
  });
}

// ------------------------------------------------------------------------------- the dialogs ----

/**
 * `9a First sync` — the merge in flight, 602×542 with its own footer.
 *
 * The split progress bar and its two labels are omitted: `SyncActivity` counts actions, not
 * directions, so nothing reports a per-direction split within a pass (#243). `about 17 minutes left`
 * is #229.
 */
export function renderFirstSync(props = {}) {
  const activity = props.activity ?? null;
  const body = fid(el("div", { class: "ob-merge" }), "mergeBody");
  body.append(fid(renderSeam({ site: "firstSync" }), "mergeSeam"));
  body.append(
    fid(el("div", { class: "ob-merge-label is-left tone-up" }, MAIN.sideLocal), "mergeLabelLeft"),
    fid(el("div", { class: "ob-merge-label is-right tone-down" }, MAIN.sideRemote), "mergeLabelRight"),
  );
  const mark = fid(
    renderHexagon({
      size: MERGE_MARK,
      state: "syncing",
      masked: true,
      numeral: props.pending ?? null,
      class: "ob-merge-mark",
    }),
    "mergeMark",
  );
  fid(mark.querySelector("defs"), "mergeMarkDefs");
  for (const [i, node] of [...mark.querySelectorAll("linearGradient")].entries()) {
    fid(node, "mergeMarkGradient", i);
    for (const [j, stop] of [...node.querySelectorAll("stop")].entries()) fid(stop, "mergeMarkStop", i, j);
  }
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "mergeMarkPath", i);
  fid(mark.querySelector("text"), "mergeNumeral");
  const title = fid(el("div", { class: "ob-merge-title" }, ONBOARDING.progressTitle), "mergeTitle");
  seamMask(title, { pad: 16 });
  body.append(mark, title);
  // `159 of 471 done` is the one part of this screen the command surface already answers.
  if (activity?.action_total != null) {
    const sub = fid(
      el(
        "div",
        { class: "ob-merge-sub" },
        ONBOARDING.progressDone(activity.action_index ?? 0, activity.action_total),
      ),
      "mergeSub",
    );
    seamMask(sub, { pad: 14, padY: 2 });
    body.append(sub);
  }
  const close = fid(el("div", { class: "ob-merge-close" }, ONBOARDING.canClose), "mergeClose");
  seamMask(close, { pad: 14, padY: 2 });
  body.append(close);

  const foot = fid(el("div", { class: "ob-merge-foot" }), "mergeFoot");
  // The sentence is about the plan the person approved, so with no plan in hand there is nothing to
  // claim and the node goes rather than rendering empty.
  const footText = mergeFooterText(props);
  // FILTERED, never `append(null)`: `Element.append` stringifies its argument, so a null child is
  // inserted as the literal text "null" — the bug app.js's own note on `replaceChildren` records,
  // and the style gate cannot see a stray text node.
  const footChildren = [
    footText ? fid(el("span", { class: "ob-merge-foot-text" }, footText), "mergeFootText") : null,
    fid(el("span", { class: "shell-spacer" }), "mergeFootSpacer"),
    fid(
      button({
        kind: "quietOutlined",
        size: "standard",
        label: MAIN.pause,
        padding: "8px 15px",
        onClick: () => props.handlers?.onPause?.(),
      }),
      "mergeFootButton",
    ),
  ].filter(Boolean);
  foot.append(...footChildren);
  return [body, foot];
}

/**
 * What a REBUILD of the merge dialog would change — and deliberately not the two numbers that move.
 *
 * The dialog layer replaces the surface's children when this string moves, and the mark inside it is
 * the syncing hexagon: `replaceChildren` restarts both travelling segments from 0%, which is the
 * failure `updateHexagon` exists to prevent and which a per-action counter in here would cause on
 * every poll. So the shape is what is in the signature and the numbers are patched by
 * `updateFirstSync`.
 */
export function firstSyncShape(props = {}) {
  return JSON.stringify([props.activity?.action_total != null, mergeFooterText(props)]);
}

/** Patch the two moving values in place, leaving the mark and its animations alone. */
export function updateFirstSync(surface, props = {}) {
  if (!surface) return;
  updateHexagon(surface.querySelector(".ob-merge-mark"), { numeral: props.pending ?? null });
  const sub = surface.querySelector(".ob-merge-sub");
  const activity = props.activity ?? null;
  if (sub && activity?.action_total != null) {
    const text = ONBOARDING.progressDone(activity.action_index ?? 0, activity.action_total);
    if (sub.textContent !== text) sub.textContent = text;
  }
}

/**
 * `nothing deleted · 2 conflicts kept as copies`, from the plan the person approved rather than from
 * a live scan: the sentence is about THIS merge, and the reviewed plan is what described it.
 */
export function mergeFooterText({ summary = null } = {}) {
  if (!summary) return "";
  const parts = [];
  if (!summary.destructive_actions) parts.push(ONBOARDING.nothingDeletedShort);
  if (summary.conflicts) parts.push(ONBOARDING.conflictsKept(summary.conflicts));
  return parts.join(" · ");
}

/**
 * `9a Consent` — the promise, after the merge.
 *
 * `12,480 files, 41.2 GB` is #207: no command reports index-wide totals, so the sub-line keeps the
 * clause that is true and drops the one that is not.
 */
export function renderConsent(props = {}) {
  const agreed = Boolean(props.agreed);
  const head = fid(el("div", { class: "ob-done" }), "doneHead");
  const mark = fid(renderHexagon({ size: CONSENT_MARK, state: "settled" }), "doneMark");
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "doneMarkPath", i);
  head.append(
    mark,
    fid(el("div", { class: "ob-done-title" }, ONBOARDING.doneTitle), "doneTitle"),
    fid(el("div", { class: "ob-done-sub" }, ONBOARDING.doneSubPhase1(props.conflicts ?? 0)), "doneSub"),
  );

  const box = checkbox({
    checked: agreed,
    label: ONBOARDING.consentCheckbox,
    onChange: (on) => props.handlers?.onAgree?.(on),
  });
  const panel = fid(
    consentPanel({ title: ONBOARDING.consentTitle, body: ONBOARDING.consentBody, footer: box }),
    "consentPanel",
  );
  fid(panel.querySelector(".band-consent-title"), "consentTitle");
  fid(panel.querySelector(".band-consent-body"), "consentBody");
  fid(panel.querySelector(".band-consent-footer"), "consentFooter");
  fid(panel.querySelector(".checkbox-box"), "consentBox");
  fid(panel.querySelector(".checkbox-label"), "consentLabel");

  const start = fid(
    button({
      kind: "primary",
      size: "bar",
      label: ONBOARDING.consentStart,
      onClick: () => props.handlers?.onStartSyncing?.(),
    }),
    "doneFootButton",
  );
  if (!agreed) setButtonKind(start, "primaryDisabled");
  const foot = fid(
    el(
      "div",
      { class: "ob-done-foot" },
      fid(el("span", { class: "ob-done-foot-text" }, ONBOARDING.consentPaused), "doneFootText"),
      fid(el("span", { class: "shell-spacer" }), "doneFootSpacer"),
      start,
    ),
    "doneFoot",
  );
  return [head, panel, foot];
}

/**
 * `9a CLI missing` — the precondition that only appears when it fails.
 *
 * The command box goes: every command in `CLI_INSTALL_COMMANDS` names a package that is in no
 * distribution's repository, by this project's own documentation, so there is nothing true to put in
 * it (#218). `Installation help` goes with it, and #231 closing does NOT bring it back: the command
 * surface can open a URL now (`open_remote` proves it), but there is still no true URL to send it to
 * — the same #218 that empties the box — and a takeover has nowhere to come back from (#244).
 */
export function renderCliMissing(props = {}) {
  const distro = props.cli?.distro ?? null;
  const row = fid(el("div", { class: "ob-cli" }), "cliRow");
  const mark = fid(
    renderHexagon({
      size: CLI_MARK,
      state: "warning",
      tone: "decision",
      flexNone: true,
      class: "ob-cli-mark",
    }),
    "cliMark",
  );
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "cliMarkPath", i);
  fid(mark.querySelector("circle"), "cliMarkDot");
  const col = fid(el("div", { class: "ob-cli-col" }), "cliCol");
  col.append(
    fid(el("div", { class: "ob-cli-title" }, ONBOARDING.cliMissingTitle), "cliTitle"),
    fid(el("div", { class: "ob-cli-body" }, ONBOARDING.cliMissingBody(distro)), "cliBody"),
    fid(
      el(
        "div",
        { class: "ob-cli-buttons" },
        fid(
          button({
            kind: "primarySoft",
            size: "standard",
            label: ONBOARDING.checkAgain,
            onClick: () => props.handlers?.onCheckCli?.(),
          }),
          "cliCheckAgain",
        ),
      ),
      "cliButtons",
    ),
  );
  row.append(mark, col);
  return [row];
}
