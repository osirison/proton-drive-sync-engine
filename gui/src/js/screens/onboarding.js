// The onboarding takeover (S7) — two steps and three surfaces around them. `09-onboarding.md`.
//
// Two steps live in the takeover body (`9a Folders`, `9a Review`); `9a CLI missing`, `9a First sync`
// and `9a Consent` are dialogs, and app.js drives which one is open. The takeover has no footer nav,
// so both steps carry an action bar with the window's own 18px bottom margin.
//
// Phase 1 omissions, each in DEVIATIONS §79 with its issue: every per-side file count and byte total
// (#240), the account line (#241), `Needs 38.4 GB free` (#206 — C4 answers the other half), the
// already-matching count (#242), the ETA (#229), the merged totals
// (#207), the install command box (#218), and four buttons with no destination — three of them
// #244 (`Add skip rules`, `See all N actions`, `Installation help`, the last also #218) and
// `Browse Proton Drive…`, which is #99.

import { el } from "../ui/el.js";
import { MAIN, ONBOARDING, PLAN, SETTINGS } from "../ui/copy.js";
import { count, outcomeOf, since } from "../ui/format.js";
import { renderHexagon, updateHexagon } from "../ui/hexagon.js";
import { renderSeam, seamMask } from "../ui/seam.js";
import { button, textInput, checkbox, setButtonKind } from "../ui/controls.js";
import { consentPanel, warnGlyph } from "../ui/bands.js";
import { renderActionBar } from "../ui/chrome.js";
import { dot, eyebrow, planActionRow } from "../ui/rows.js";
// THE PLAN SCREEN'S OWN MODEL AND ROW GRAMMAR (#244), imported rather than rewritten: the actions
// detour draws the same rows the Plan screen draws, and a second copy of "which mark, which side,
// which outcome" is how two surfaces come to disagree about one plan. `summarise` brings #324's
// rule with it — the counts are the daemon's, the rows are the window's.
import { isDisplayDestructive, isGated, markOf, pathOf, summarise } from "./plan.js";
// THE ONE PLACE THE `CAN'T BE SYNCED` GROUP IS DECIDED, imported rather than re-derived (#315).
// It owns the membership rule (`remote_not_downloadable` is excluded — a Docs file is a real file
// on Proton Drive, not a non-file in your folder), the noun for each reason token, and the counted
// kinds phrase. `tray.js` borrows `transfersOf` from `main.js` on exactly this argument: a second
// copy of a group's definition is how two screens come to disagree about one set.
import { cannotSyncFrom } from "./activity.js";
import { fid } from "../fixtures/frames.js";

/** The marks, at the five sizes this flow draws them. 80 is two-valued in `strokeForSize`. */
const REVIEW_MARK = 80;
const REVIEW_MARK_STROKE = 4.6;
const FACT_MARK = 13;
const MERGE_MARK = 116;
const CONSENT_MARK = 76;
const CLI_MARK = 34;

// ------------------------------------------------------------------------------- the model ----

/** The two sub-screens the takeover can detour into (#244), and the only values `detour` takes. */
export const DETOURS = ["skip", "actions"];

/**
 * What the flow is showing. `checking` outranks a payload for the same reason `5a Checking` does:
 * the plan on screen is the old one.
 *
 * A DETOUR OUTRANKS EVERYTHING, and it is its own arm rather than a fall-through (#244). It is a
 * place the person asked to be, so nothing the daemon does may move them off it; and left to fall
 * through, a detour opened from step 1 would land in the `folders` arm and draw the step it was
 * opened from. The step itself does not change while a detour is open — which is the whole answer
 * to "return to the right step": there is no step to restore, only a sub-screen to close.
 */
export function bodyOf({
  step = "folders",
  detour = null,
  dryRun = null,
  checking = false,
  error = null,
} = {}) {
  if (DETOURS.includes(detour)) return detour;
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
  // The remote path is EDITABLE and the local one is not: nothing browses Proton Drive, so there is
  // no picker to open (S6 settled the same asymmetry on `8a Settings`). #99 gave the daemon a `list`
  // verb that could feed one; the screen for it is unbuilt, and #311 removed the GUI's own
  // CLI-shelling `list_remote` rather than leave a second, ungated client behind a button nobody
  // draws.
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

  // The skip prompt has its button back (#244). It opens a sub-screen INSIDE the takeover rather
  // than the Settings tab it names: leaving for Settings was the one-way door — on a machine with
  // no daemon the main screen offers `Try again now` and nothing that resumes setup. The sentence
  // keeps its "or any time later in Settings", which is still true and is now the second way in
  // rather than the only one.
  const skip = fid(el("div", { class: "ob-skip" }), "skipPanel");
  skip.append(
    fid(warnGlyph(), "skipGlyph"),
    fid(el("div", { class: "ob-skip-text" }, ONBOARDING.skipHint), "skipText"),
    // The frame's own button, at its own measurements: 12.5px in `--text-3` on a `--border`
    // hairline, 9px radius, 8/15 padding (`9a Folders`, `div[1]/div[2]/button`).
    fid(
      button({
        kind: "secondary",
        size: "standard",
        label: ONBOARDING.addSkipRules,
        padding: "8px 15px",
        radius: "var(--r-9)",
        fontSize: "12.5px",
        class: "ob-skip-button",
        onClick: () => props.handlers?.onDetour?.("skip"),
      }),
      "skipButton",
    ),
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
export function factRows(summary, cannotSync = []) {
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
  // `3 files can't be synced — a socket and two shortcuts`, BOTH HALVES FROM ONE SOURCE (#315).
  //
  // `DryRunReport.cannot_sync` is the plan's own local stat-walk reporting what it dropped — a
  // socket, a symlink, a FIFO, a device node (#232) — carried on the report since PR #318, and it
  // arrives the same way down both paths this screen can take: the child `proton-syncd --dry-run`
  // (onboarding, before any daemon exists) parses it out of the report, and the daemon's `plan`
  // verb copies `ReviewedPlan.cannot_sync` onto the same field.
  //
  // `PlanSummary.skipped_unsupported` USED TO BE THIS NUMBER AND IS DELIBERATELY NOT ANY MORE. It
  // counts `SkipUnsupported` plan rows, which are overwhelmingly *remote* nodes the CLI cannot
  // fetch — a Proton-native Docs or Sheets file. Summing the two would put one sentence over two
  // sets, and naming the kinds of one beside the count of the other is worse. The Activity screen
  // reached the same verdict from the other side and already ships it: `cannotSyncFrom` EXCLUDES
  // `remote_not_downloadable` from this group entirely, because a Docs file is a real file on
  // Proton Drive rather than a non-file in your folder. So this row is silent about it, and the
  // count survives where it means something — S5's counters, and `actionsThatHappen` above, which
  // still subtracts those rows from `See all N actions`.
  const cannot = cannotSyncFrom(cannotSync);
  if (cannot.count) {
    rows.push({
      at: 2,
      tone: "inert",
      label: ONBOARDING.cannotSync(cannot.count, cannot.kinds),
      note: ONBOARDING.skipped,
      dim: true,
    });
  }
  if (summary && !summary.destructive_actions) {
    rows.push({ at: 3, tone: "hexagon", label: ONBOARDING.nothingDeleted, note: ONBOARDING.eitherSide });
  }
  return rows;
}

function factsBlock(summary, cannotSync) {
  const rows = factRows(summary, cannotSync);
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
  // `#[serde(default, skip_serializing_if = "Vec::is_empty")]` on the engine side, so a report with
  // nothing to say here omits the key entirely — including every report a build older than #318
  // emitted. Absent is empty, and empty draws no row.
  const cannotSync = props.dryRun?.report?.cannot_sync ?? [];

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
  const facts = factsBlock(summary, cannotSync);
  if (facts) body.append(facts);
  // `See all N actions` opens the plan INSIDE the takeover (#244). The Plan screen itself is behind
  // a footer door the takeover covers, and leaving for it would be the same one-way door the skip
  // panel's button was; the sub-screen is the same list of rows with a `Back` into this step.
  // `about 25 minutes to finish` is still #229.
  //
  // Drawn on the timing row whether or not the timing text is: the row is a flex line with a
  // spacer, and a rehearsal that has not stamped `checkedAt` still has a plan to look at. `9a
  // Review` draws both, which is the state the gate compares.
  const actions = actionsThatHappen(summary);
  if (props.checkedAt != null || actions != null) {
    const timing = fid(el("div", { class: "ob-timing" }), "timing");
    if (props.checkedAt != null) {
      timing.append(
        fid(
          el("span", { class: "ob-timing-text" }, ONBOARDING.workedOutPlain(since(props.checkedAt))),
          "timingText",
        ),
      );
    }
    timing.append(fid(el("span", { class: "shell-spacer" }), "timingSpacer"));
    if (actions != null) {
      // The frame's measurements again (`9a Review`, `div[1]/div[2]/button`): 12px, 7/14, radius 8.
      timing.append(
        fid(
          button({
            kind: "secondary",
            size: "standard",
            label: ONBOARDING.seeAllActions(actions),
            padding: "7px 14px",
            radius: "var(--r-8)",
            fontSize: "12px",
            onClick: () => props.handlers?.onDetour?.("actions"),
          }),
          "timingButton",
        ),
      );
    }
    body.append(timing);
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

// ------------------------------------------------------------------------ the two detours ----
//
// #244's answer, and the shape of it is the point: a sub-screen INSIDE the takeover, with a `Back`
// that closes it. The takeover covers everything and cannot be dismissed, which is what makes it
// reliable on a fresh machine — so a button that left for Settings or for the Plan door would be a
// one-way door out of setup, on exactly the machine where nothing else can bring you back.
//
// Neither is drawn by any frame, so both are built from shapes the deck already has: the two steps'
// own title block, S6's rule rows, S4's action rows. No slot is stamped in either.

/** One heading pair, in the takeover's own type. */
function detourTitle(title, sub) {
  return el(
    "div",
    { class: "ob-title-block" },
    el("div", { class: "ob-title" }, title),
    el("div", { class: "ob-sub" }, sub),
  );
}

/**
 * `Add skip rules` — onboarding's own skip-rules editor.
 *
 * STAGED, NOT WRITTEN. The rules go into the config with the folder pair when `See what will
 * happen` writes it (`app.js`'s `onNext`), which is what keeps the flow's one promise — nothing is
 * written until you continue — and what makes the rehearsal on the next screen a rehearsal OF these
 * rules. Settings' own tab is the other way in, and stays: the panel's sentence still names it.
 */
function skipBody(props) {
  const rules = props.skipRules ?? [];
  const body = el("div", { class: "ob-detour" });
  const list = el("div", { class: "ob-rules" });
  if (rules.length) {
    for (const pattern of rules) {
      const row = el("div", { class: "ob-rule" });
      row.append(
        el("span", { class: "ob-rule-pattern mono" }, pattern),
        el("span", { class: "shell-spacer" }),
        button({
          kind: "secondary",
          label: SETTINGS.remove,
          padding: "6px 13px",
          radius: "var(--r-8)",
          fontSize: "12px",
          onClick: () => props.handlers?.onRemoveSkipRule?.(pattern),
        }),
      );
      list.append(row);
    }
  } else {
    list.append(el("div", { class: "ob-rules-empty" }, ONBOARDING.noSkipRules));
  }
  // The add row. The FIELD'S VALUE IS THE FLOW'S, not the DOM's: this body is rebuilt on the ~2s
  // status poll, so a draft living in the input would be retyped from empty twice a second — the
  // caret bug S6 and step 1 both already carry a note about. `signatureOf` therefore leaves the
  // draft out (so a keystroke does not rebuild) and keeps the rules in (so `Add` redraws the list).
  const field = textInput({
    value: props.skipDraft ?? "",
    class: "input ob-rule-input",
    placeholder: SETTINGS.addRulePlaceholder,
    "aria-label": SETTINGS.addRulePlaceholder,
    onInput: (e) => props.handlers?.onSkipDraft?.(e.target.value),
  });
  const add = el(
    "div",
    { class: "ob-rule-add" },
    field,
    button({
      kind: "secondaryFilled",
      label: SETTINGS.add,
      padding: "9px 16px",
      radius: "var(--r-10)",
      fontSize: "12.5px",
      onClick: () => props.handlers?.onAddSkipRule?.(),
    }),
  );
  body.append(list, add, el("div", { class: "ob-detour-note" }, ONBOARDING.nothingUntilApproved));
  return [detourTitle(ONBOARDING.skipTitle, SETTINGS.skipIntro), body];
}

/**
 * `See all N actions` — the plan the review screen summarises, row by row.
 *
 * THE SAME N AS THE BUTTON. `actionsThatHappen` is the count both read, and the rows are filtered by
 * the same fact: a `skip_unsupported` row is a thing that will NOT happen, so it is not in a list
 * called every action and is not in the number above it. The window's own truncation is named at
 * the foot, from the daemon's count, exactly as the Plan screen does it (#319/#324).
 */
export function actionsModel(dryRun) {
  const report = dryRun?.report ?? null;
  const model = summarise(report?.plan ?? [], report?.summary ?? null);
  const rows = model.rows.filter((row) => row.action !== "skip_unsupported");
  const total = actionsThatHappen(report?.summary) ?? rows.length;
  return {
    rows,
    total,
    conflicts: model.conflicts,
    gated: model.gated.length,
    // Against the SAME total the head claims, so the list and the sentence above it cannot describe
    // different sets. Never negative: a plan whose summary undercounts its own rows draws no line.
    hidden: Math.max(0, total - rows.length),
  };
}

function actionsBody(props) {
  const model = actionsModel(props.dryRun);
  const rows = model.rows;
  const total = model.total;
  const body = el("div", { class: "ob-detour" });
  const head = el(
    "div",
    { class: "ob-actions-head" },
    eyebrow({ tone: "neutral", text: PLAN.everyAction }),
    el("span", { class: "shell-spacer" }),
    el("span", { class: "ob-actions-count" }, PLAN.actionSummary(total, model.conflicts)),
  );
  const list = el("div", { class: "ob-actions" });
  for (const row of rows) {
    const { glyph, tone } = markOf(row.action);
    list.append(
      planActionRow({
        glyph,
        tone,
        path: pathOf(row),
        outcome: outcomeOf(row.action, "plan"),
        tinted: isDisplayDestructive(row.action),
        destructive: isGated(row.action),
      }),
    );
  }
  if (model.hidden > 0) list.append(el("div", { class: "ob-actions-more" }, MAIN.andMore(model.hidden)));
  body.append(head, list);
  // `5a Plan`'s own second line, which is the same sentence about the same thing: this is a
  // rehearsal, and it says how many of its rows cannot be undone — nought, on a first sync.
  return [detourTitle(ONBOARDING.actionsTitle, PLAN.sub(model.gated)), body];
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
  // THE RULES AND NOT THE DRAFT, the same asymmetry one screen along (#244): `Add` has to redraw the
  // list, and a keystroke in the add field must not — this body is rebuilt on the ~2s poll and a
  // rebuilt `<input>` loses the caret and everything typed since the last one.
  if (body === "skip") return JSON.stringify(["skip", props.skipRules ?? []]);
  // The plan's rows, by the same rule the Plan screen's own signature uses.
  if (body === "actions") {
    return JSON.stringify([
      "actions",
      (props.dryRun?.report?.plan ?? []).map((row) => [row.path, row.destination_path, row.action]),
      props.dryRun?.report?.summary?.total ?? null,
    ]);
  }
  if (body === "checking") return "checking";
  if (body === "failed") return JSON.stringify(["failed", props.error]);
  const s = props.dryRun?.report?.summary ?? null;
  return JSON.stringify([
    "review",
    s && [s.uploads, s.downloads, s.conflicts, s.skipped_unsupported, s.destructive_actions],
    // The `can't be synced` row's own source (#315), and it must be in here or the row goes stale:
    // `skipped_unsupported` above no longer draws anything, so a rehearsal that gained or lost a
    // socket would leave the previous sentence on screen. The KINDS and not just the count — one
    // socket replaced by one symlink is the same number and a different sentence.
    cannotSyncFrom(props.dryRun?.report?.cannot_sync ?? []).kinds,
    props.freeSpace?.available ?? null,
    props.checkedAt == null ? null : since(props.checkedAt),
  ]);
}

/** The takeover body, as window-root siblings — the seam's `left:50%` resolves against the window. */
export function renderOnboarding(props = {}) {
  const nodes = (() => {
    switch (bodyOf(props)) {
      case "skip":
        return skipBody(props);
      case "actions":
        return actionsBody(props);
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
  // A DETOUR'S OWN ARM, before anything else and never by falling through (#244). `Back` here means
  // "close this sub-screen", which is a different destination from step 2's `Back` (that one goes
  // to step 1 and re-runs the rehearsal) — and the arm below would have given it the right SHAPE
  // with the wrong handler, which is the kind of accident a fall-through hands you.
  if (DETOURS.includes(body)) {
    const bar = renderActionBar({
      consequence: fid(
        button({
          kind: "quietOutlined",
          size: "bar",
          label: ONBOARDING.back,
          padding: "11px 18px",
          onClick: () => handlers.onCloseDetour?.(),
        }),
        "barBack",
      ),
      bottom: 18,
    });
    fid(bar.querySelector(".shell-spacer"), "barSpacer");
    return fid(bar, "bar");
  }
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
 * The split progress bar and its two counts (#243).
 *
 * ONE TRACK, TWO FILLS, AND THE FILLS ARE COMPUTED. `activity.pass` carries `uploaded_files` and
 * `downloaded_files` — transfers that landed and committed this pass, per direction — so each fill
 * is that count over `action_total`, the same denominator as `159 of 471 done` one line above it.
 * The bar's filled portion is therefore the transfers among the actions that line counts, and it can
 * never claim more than that line does: `uploaded_files + downloaded_files <= action_index`.
 *
 * NOT DRAWN AT THE FRAME'S WIDTHS, and that is a recorded decision rather than a miss. `9a First
 * sync` paints the fills 48px and 88px of a 400px track while labelling them `44 sent` and
 * `115 received` — 48:88 is 0.55 and 44:115 is 0.38, and no denominator produces both, so the two
 * halves of the drawing disagree with each other. The total is right (48+88 = 136 of 400 = 34%,
 * against 159/471 = 33.8%), so it is the split that is hand-drawn. Reproducing it would mean
 * ignoring the numbers the same block prints. DEVIATIONS §63b.
 *
 * `null` when nothing is known yet: no `pass` block (a daemon predating #213) or no `action_total`
 * means the block is omitted rather than drawn empty, which is the same rule the sub-line above
 * follows.
 */
function mergeProgress(activity) {
  const pass = activity?.pass;
  const total = activity?.action_total;
  if (!pass || !total) return null;
  const width = (files) => `${Math.min(100, (files / total) * 100)}%`;
  const up = el("div", { class: "ob-merge-fill-up" });
  const down = el("div", { class: "ob-merge-fill-down" });
  up.style.width = width(pass.uploaded_files ?? 0);
  down.style.width = width(pass.downloaded_files ?? 0);
  const track = fid(el("div", { class: "ob-merge-track" }, [up, down]), "mergeTrack");
  fid(up, "mergeFillUp");
  fid(down, "mergeFillDown");
  const counts = fid(
    el("div", { class: "ob-merge-counts" }, [
      fid(
        el("span", { class: "ob-merge-count-up" }, ONBOARDING.sentCount(pass.uploaded_files ?? 0)),
        "mergeCountUp",
      ),
      fid(
        el("span", { class: "ob-merge-count-down" }, ONBOARDING.receivedCount(pass.downloaded_files ?? 0)),
        "mergeCountDown",
      ),
    ]),
    "mergeCounts",
  );
  // NO `seamMask`, unlike every other block in this dialog: the frame records this one with no
  // background at all. The seam runs behind it and the design lets it — a 3px track and two short
  // labels at the outer edges leave it visible down the middle, which is the point of drawing the
  // merge over a seam in the first place.
  return fid(el("div", { class: "ob-merge-progress" }, [track, counts]), "mergeProgress");
}

/**
 * `9a First sync` — the merge in flight, 602×542 with its own footer.
 *
 * `about 17 minutes left` is still omitted (#229). The split progress bar is drawn — see
 * `mergeProgress` for what it derives its two fills from and where it departs from the drawing.
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
  // FILTERED, never `append(null)` — `Element.append` stringifies its argument, so a null child
  // lands as the literal text "null" (the same trap the footer below records).
  const progress = mergeProgress(activity);
  if (progress) body.append(progress);
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
