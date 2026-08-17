// The settings screen (S6) — what syncs, how often, and how much say you get. `08-settings.md`.
//
// FOUR TABS AND FIVE FRAMES, AND ONLY THREE OF THE TABS ARE DRAWN. `8a Settings` is Folders,
// `8a Skip rules` is What to skip, `8a Deletions tab` is Deletions (a 600px re-render, not a cut-out
// of the window — see `fids.js`), `8a Schedule monthly` is one panel of Folders in its other state,
// and `8a Save refused` is a dialog over any of them. Advanced is specified in prose and drawn
// nowhere.
//
// EVERY CONTROL HERE WRITES SOMETHING, and that is the line this screen is built along. The old
// screen was the config file with labels; this one says what each setting does to your files and
// puts the key underneath in mono. A control that cannot write is not drawn — the same call
// #224 and #227 record, and this screen would have been the third and fourth place to make it.
//
// WHAT THE FRAMES DRAW THAT PHASE 1 CANNOT, with the issue that closes each:
//
//   · the whole full-sweep SCHEDULE — Weekly/Monthly, seven day chips, a time stepper and
//     `full_scan_schedule · weekly sun 03:00` (G4 #193). There is no such key, no scheduler and no
//     command. `IMPLEMENTATION-PLAN.md` §4 says to present `scan_interval` in plain language inside
//     the same panel shell, so the panel keeps its shell and changes its subject — and its TITLE
//     with it, because `Compare everything, top to bottom` over a timer that (with live updates on)
//     runs an incremental pass would be a false claim about what happens to someone's files. This
//     is the largest single deviation in the design-v2 build. DEVIATIONS §78.
//   · `12,480 files, 41.2 GB in here today` under the local folder, and `A full check of all 12,480
//     files` in the panel above (G7 #207). `skip_rule_usage` counts files and not bytes, so half of
//     it would still be missing; both clauses go and the sentences that remain are the ones that
//     say what changing the setting does.
//   · `Takes about 4 minutes … Last one 2 days ago` under `Full sweep now` (G24 #238) — no per-pass
//     duration exists, and nothing records which past pass was a full sweep.
//   · `added 14 Jul` on a rule — a TOML array of globs carries no per-entry timestamps. Not a
//     missing command: an absent fact, and the fixture pins it as a literal that nothing reads.
//   · (WAS) the unsyncable panel — `Two more files can't be synced no matter what` and `See them`
//     (G19 #232). Drawn since #232: the files it counts still never enter the index, and the panel
//     does not read one — `ControlResponse.unsyncable` is the daemon's standing list of what its
//     own local walk drops, and `See them` opens the `7a Never synced` group fed by the same list.
//   · `That folder doesn't exist on Proton Drive` and `Create it on Proton Drive` on the refusal
//     (G22 #236). `write_config` validates TOML and never contacts Proton Drive, so it cannot know.
//   · four of Advanced's six settings (G23 #237): the socket path, the log level, the conflict
//     suffix and *Reset the index* have no key and no command between them.
//
// AND TWO THINGS THE SCREEN GAINED RATHER THAN LOST. `Sweep now` is `ControlCommand::Resync`, which
// the daemon has had since #160 and no command exposed; `Choose…` is a real folder picker. Both are
// recorded in `commands.rs`'s module note, because a screen adding to the command surface is the
// exception and not the rule.

import { el } from "../ui/el.js";
import { MAIN, NOTIFY, SETTINGS } from "../ui/copy.js";
import { button, pillTabs, radioCard, setButtonKind, stepper, textInput, toggle } from "../ui/controls.js";
import { eyebrow, splitEmphasis } from "../ui/rows.js";
import { renderSeam } from "../ui/seam.js";
import { renderHexagon } from "../ui/hexagon.js";
import { fid } from "../fixtures/frames.js";

/**
 * The tabs, in the drawn order. The index is the frame's `div[1]/button[i]`.
 *
 * A FIFTH PILL THE 8a FRAMES DO NOT DRAW (S9). `08-settings.md` enumerates four and
 * `11-notifications.md` specifies a settings surface with no home; the `11a Settings` frame's own
 * caption calls it "the settings tab", `14-behaviour-and-state.md`'s fallback row calls the section
 * "Notifications", and the `11a Settings` card is drawn with the same chrome as the `8a Deletions
 * tab` crop — same background, border, radius, padding and presentation shadow. So it is a tab.
 *
 * It costs the gate nothing: the drawn pill row is 1040 wide and left-aligned, so a fifth pill moves
 * none of the four boxes `8a Settings` asserts. DEVIATIONS §83.
 */
const TABS = [
  { id: "folders", label: SETTINGS.tabs.folders },
  { id: "skip", label: SETTINGS.tabs.skip },
  { id: "deletions", label: SETTINGS.tabs.deletions },
  { id: "notifications", label: NOTIFY.settings.tab },
  { id: "advanced", label: SETTINGS.tabs.advanced },
];

/**
 * The three `notify_policy` values, in the order the cards are drawn.
 *
 * GUI-LOCAL, not a daemon key (IMPLEMENTATION-PLAN row 6). The daemon parses its config with
 * `deny_unknown_fields`, so writing `notify_policy` into `proton-sync.toml` would stop it starting —
 * it lives in the GUI's own `gui.toml` beside it. `never` must not change engine behaviour: the
 * deletion queue still holds, because nothing here is passed to the daemon at all.
 */
export const NOTIFY_POLICIES = [
  { id: "only_when_needed" },
  { id: "only_permanent_deletions" },
  { id: "never" },
];

/** `Choose…`, `Add` — a filled secondary at input height. */
const inputButton = (label, onClick, padding) =>
  button({ kind: "secondaryFilled", label, onClick, padding, radius: "var(--r-10)", fontSize: "12.5px" });

/** `Remove` — the quietest button on the screen, and the only one that stages a loss. */
const rowButton = (label, onClick) =>
  button({ kind: "secondary", label, onClick, padding: "6px 13px", radius: "var(--r-8)", fontSize: "12px" });

/** The stepper's two buttons, named so a repeated press keeps the keyboard on the one being used. */
function namedSteps(node) {
  const [minus, plus] = node.querySelectorAll(".btn");
  focusable(minus, "interval-down");
  focusable(plus, "interval-up");
  return node;
}

/**
 * Name a control so focus can find it again.
 *
 * THIS SCREEN IS A FORM AND IT IS REBUILT TWICE A SECOND. Every handler here ends in a render and
 * the status poll rebuilds the body anyway, so a control that is focused when that happens is
 * removed from the document and the keyboard lands on `<body>`: Tab restarts at the top, Space does
 * nothing, and there are twenty-one such controls on this screen against five text fields. The
 * review measured it — `focused before: radio: Never ask` / `after poll: BODY`.
 *
 * `data-sfocus` is matched by scanning rather than by a selector, so an id may contain anything a
 * skip pattern can. `data-focus-key` is a SELECTOR (`focusKeyOf` in app.js hands it to
 * `querySelector`), so only ids that are safe inside one carry it — that is what lets a dialog
 * opened over this screen put focus back on the field it was opened from.
 */
function focusable(node, id) {
  if (!node) return node;
  node.dataset.sfocus = id;
  if (/^[a-z0-9:_-]+$/i.test(id)) node.dataset.focusKey = `[data-sfocus="${id}"]`;
  return node;
}

// ------------------------------------------------------------------------------ the model ----

/**
 * The three deletion policies, in the order the cards are drawn, and what each one writes.
 *
 * BOTH BOOLEANS, ALWAYS. `remote` gates the recoverable direction (a file leaving this computer
 * lands in Proton's Trash) and `local` the permanent one, and a write that set only one of them
 * could leave a pair no card describes. DEVIATIONS §68.
 */
export const POLICIES = [
  { id: "ask_every_time", remote: true, local: true },
  { id: "only_permanent", remote: false, local: true },
  { id: "never", remote: false, local: false },
];

/**
 * Which card is selected, or `null` for the fourth combination.
 *
 * `remote: true, local: false` — ask before sending a delete to Proton's Trash, but wipe local files
 * for good without asking — is reachable by hand-editing the config and is a live safety policy with
 * no card. **It draws no selection rather than the nearest one**: coercing it would mean the next
 * save silently rewrote a setting nobody touched, in the one screen built not to do that. §68.
 *
 * READ OFF `deletion_policy`, WHICH IS NOW A KEY AND NOT A DERIVATION (#194). It used to be derived
 * from the two booleans here, and deriving it here again would be the second copy: the daemon
 * accepts `deletion_policy = "never"` as a key of its own, and a file written that way reports both
 * booleans as absent — so the old reading drew `Ask me every time` over a machine that asks about
 * nothing. `read_config` resolves whichever spelling the file uses and this reads its answer.
 *
 * The booleans remain the fallback for a payload that predates the field. An absent key means
 * `true`, which is what stops an empty config drawing as `Never ask` on a machine that is in fact
 * asking about everything.
 */
export function policyOf(config) {
  if (!config) return null;
  const resolved =
    config.deletion_policy ??
    POLICIES.find(
      (p) =>
        p.remote === (config.delete_approval_remote ?? true) &&
        p.local === (config.delete_approval_local ?? true),
    )?.id ??
    // The fourth combination has no card and no id in `POLICIES`; naming it here keeps the lookup
    // below total, so an unknown value and the undrawn one both answer `null` the same way.
    "only_recoverable";
  return POLICIES.some((p) => p.id === resolved) ? resolved : null;
}

/**
 * What a rule's two lines say. The whole point of the tab is here: an active rule names what it is
 * hiding, a stale one says it is safe to remove, and nothing in between claims either.
 *
 * §69a left this to S6 and the frame gives two of the five answers. The discriminator is the rule's
 * FOLDER ANCHOR plus whether the samples are the whole set:
 *
 *   · a bare glob (`*.tmp`, `folder_exists: null`) makes no claim about a folder, so it names the
 *     files — but only when `samples` holds all of them. The command caps samples at four
 *     (`MAX_SAMPLES`), and a list of four under `Skipping 50 files right now` reads as the full set.
 *   · anything anchored at a folder describes itself by size, which is what `video-raw/**` draws.
 *
 * AND `safe to remove` IS NEVER SAID OF A RULE THAT IS HIDING SOMETHING. It needs both halves —
 * nothing matched AND the folder is known gone (§69b) — because removing a rule that still matches
 * files starts syncing them, and this is the sentence someone acts on without checking.
 */
export function ruleEffect(rule) {
  // NOT MEASURED IS NOT MEASURED ZERO, and this is the branch the review caught reading the other
  // way. `skip_rule_usage` walks the local tree, is fired unawaited on every visit, and can fail —
  // so for the length of the walk, and permanently after an error, the report has nothing to say
  // about a rule that may be hiding forty gigabytes. `rule.files ?? 0` turned that into
  // `Matching nothing`, which is this project's own "unknown is never zero" rule broken on the one
  // sentence that invites someone to delete a rule.
  if (!rule) return null;
  // The walk could not evaluate this pattern. Its own words, in mono (voice rule 4) — and no
  // counts, because a rule that could not be checked has none.
  if (rule.error) return { effect: SETTINGS.ruleUnchecked, detail: rule.error, dim: false };
  // A row the report never answered for. `files` absent — not zero — is what says so.
  if (rule.files == null) return { effect: SETTINGS.ruleChecking, detail: null, dim: false };
  const files = rule.files;
  if (files > 0) {
    const named = rule.folder_exists == null && (rule.samples ?? []).length === files;
    return {
      effect: named ? SETTINGS.skippingNow(files) : SETTINGS.skippingSize(files, rule.bytes ?? 0),
      detail: named
        ? rule.samples.map((s) => s.path).join(", ")
        : rule.folder_exists === true
          ? SETTINGS.ruleFolderHere
          : null,
      dim: false,
    };
  }
  if (rule.folder_exists === false) {
    return { effect: SETTINGS.matchingNothing, detail: SETTINGS.staleRule, dim: true };
  }
  // Matching nothing with its folder still there, or with no folder to have an opinion about. Idle,
  // not stale — and NOT dimmed: the row is doing its job on files that are not there today.
  return {
    effect: SETTINGS.matchingNothing,
    detail: rule.folder_exists === true ? SETTINGS.ruleFolderHere : null,
    dim: false,
  };
}

/**
 * `hiding 4 files, 3.1 GB in total`, or the hedged form when the walk could not read everything.
 *
 * `skip_rules.rs` carries `unreadable_directories`/`unreadable_entries` for exactly this: with
 * either above zero every number on the tab is a floor, and the tab must not present a floor as a
 * fact. Undrawn — no frame has an unreadable folder in it.
 */
export function totalLine(report) {
  if (!report) return null;
  const partial = (report.unreadable_directories ?? 0) > 0 || (report.unreadable_entries ?? 0) > 0;
  const line = partial ? SETTINGS.hidingFloor : SETTINGS.hidingTotal;
  return line(report.total_files ?? 0, report.total_bytes ?? 0);
}

/**
 * The footer's amber cost line — `One rule removed — 2 files, 3.1 GB will start syncing.`
 *
 * `unique_files`/`unique_bytes`, NOT `files`/`bytes`: removing a rule only starts syncing the files
 * no OTHER rule still hides, and `will start syncing` is a promise about what happens next. The two
 * agree on the drawn frame because its three rules do not overlap, which is exactly why the
 * distinction has to be in the code rather than discovered by whoever first writes two that do.
 *
 * One rule only, because the sentence says `One rule removed`. Remove two and the neutral note
 * stands: a wrong number is worse than no number, and the deck has no plural form to reach for.
 */
export function removalCost(saved, staged, report) {
  if (!saved || !staged) return null;
  const gone = saved.filter((p) => !staged.includes(p));
  if (gone.length !== 1) return null;
  const rule = (report?.rules ?? []).find((r) => r.pattern === gone[0]);
  // AMBER MEANS A COST, so a removal that has none keeps the neutral note. `08-settings.md` is
  // explicit — the line appears "when a pending change has a cost" — and removing a rule that is
  // hiding nothing is the commonest safe edit on this tab: it is what `safe to remove` invites.
  // `One rule removed — 0 files, 0 B will start syncing.` is both true and a false alarm.
  if (!rule || (rule.unique_files ?? 0) === 0) return null;
  return SETTINGS.ruleRemovedCost(rule.unique_files, rule.unique_bytes ?? 0);
}

/**
 * The `ConfigUpdate` for a save: the fields that CHANGED, and nothing else.
 *
 * `Saving writes only what you changed` is a wire contract and not just footer copy. `write_config`
 * edits the TOML in place, so a field sent as `Some` is a key written — send everything and a save
 * materialises `proton_timeout_secs = 60` in a file that never had the line, which is the opposite
 * of what the note promises. Only the diff goes.
 *
 * Arrays compare by content: `exclude` is staged as a new array on every edit, so identity would
 * report a change that removing and re-adding the same rule did not make.
 */
export const ABSENT_DEFAULTS = {
  // `src/config.rs` resolves this to 300 when the file is silent, and `timerPanel` draws that —
  // so stepping the timer up and back down to what it already said must not stage a change.
  scan_interval_secs: 300,
  events_driven: true,
  delete_approval_remote: true,
  delete_approval_local: true,
  deletion_policy: "ask_every_time",
};

export function configUpdate(config, edits) {
  const same = (a, b) =>
    Array.isArray(a) && Array.isArray(b) ? a.length === b.length && a.every((v, i) => v === b[i]) : a === b;
  const update = {};
  for (const [key, value] of Object.entries(edits ?? {})) {
    // AN ABSENT KEY IS ITS DEFAULT, not a difference from it. `read_config` returns `null` for a
    // key the file does not have and the screen draws the daemon's default in its place — so on a
    // fresh config, clicking the card that is already selected staged `true` against `null` and
    // marked the screen dirty, and saving materialised two keys the file never had. That is the
    // opposite of what the footer promises. Only the keys whose drawn value comes from a default
    // are listed: everything else is drawn empty when absent, and setting it IS a change.
    const current = config?.[key] ?? (key in ABSENT_DEFAULTS ? ABSENT_DEFAULTS[key] : config?.[key]);
    if (!same(value, current)) update[key] = value;
  }
  return update;
}

/** Anything staged that the loaded config does not already say. */
export const isDirty = (config, edits) => Object.keys(configUpdate(config, edits)).length > 0;

/**
 * The daemon's reason, without the sentence `write_config` wraps it in.
 *
 * `ConfigError::Display` produces `config would be rejected by the daemon: <reason>` and the frame
 * draws the bare reason in its mono box. Voice rule 4 says the daemon's words are quoted exactly —
 * which is what makes the prefix worth removing rather than leaving: the prefix is the GUI's
 * sentence about the daemon's words, and the dialog's own body already says it.
 */
export function refusalReason(message) {
  if (!message) return null;
  const at = message.indexOf(": ");
  return message.startsWith("config would be rejected by the daemon") && at >= 0
    ? message.slice(at + 2)
    : message;
}

/**
 * `scan_interval_secs`, in the plain language `IMPLEMENTATION-PLAN.md` §4 asks for.
 *
 * Whole minutes read as minutes; anything else reads in seconds rather than rounding. A config
 * written by hand at 90s must not draw as `2 min` — the stepper would then write 120 for a value
 * nobody touched, which is the same silent rewrite `policyOf` refuses one block up.
 */
export function intervalLabel(secs) {
  const n = Number(secs);
  if (!Number.isFinite(n) || n <= 0) return SETTINGS.timerSeconds(0);
  return n % 60 === 0 ? SETTINGS.timerUnit(n / 60) : SETTINGS.timerSeconds(n);
}

/** One minute a step, and never below one: a zero interval is a daemon in a spin loop. */
export const MIN_INTERVAL_SECS = 60;
export const MAX_INTERVAL_SECS = 7200;
export const stepInterval = (secs, delta) =>
  Math.min(MAX_INTERVAL_SECS, Math.max(MIN_INTERVAL_SECS, Number(secs || 0) + delta * 60));

// ------------------------------------------------------------------------ shared furniture ----

/** A settings panel: the bordered, padded box every group on this screen sits in. */
const panel = (cls, children) => el("div", { class: `settings-panel ${cls}` }, children);

/** A panel's title and its 12.5px sub-line. The sub is omitted, never blank — §4's rule. */
const panelText = (title, sub) =>
  el(
    "div",
    { class: "settings-panel-text" },
    el("div", { class: "settings-panel-title" }, title),
    sub ? el("div", { class: "settings-panel-sub" }, sub) : null,
  );

/** The mono key line under a control. Not deck copy — it is the config key, verbatim. §68. */
const keyLine = (text) => el("div", { class: "settings-key" }, text);

// -------------------------------------------------------------------------- tab 1 · folders ----

/**
 * One side of the folder pair.
 *
 * The two sides are NOT mirrors: the left has a `Choose…` button and the right does not, because a
 * remote folder cannot be browsed for — no screen browses Proton Drive, and the daemon's `list`
 * verb (#99) has no picker in front of it.
 * The right side's input carries the whole width and right-aligns its text instead, which is what
 * makes the pair read as two ends of one line rather than as two fields.
 */
function pairSide(props, side) {
  const local = side === 0;
  const { config, handlers } = props;
  const input = fid(
    textInput({
      value: (local ? config.local_root : config.remote_root) ?? "",
      mono: true,
      class: local ? "input is-mono" : "input is-mono settings-pair-input-remote",
      "aria-label": local ? MAIN.sideLocal : MAIN.sideRemote,
      // The caret survives the 2s rebuild by NAME — app.js finds the field again through this.
      "data-field": local ? "local_root" : "remote_root",
      "data-sfocus": local ? "field:local_root" : "field:remote_root",
      "data-focus-key": local ? '[data-sfocus="field:local_root"]' : '[data-sfocus="field:remote_root"]',
      onInput: (e) => handlers.onRoot?.(local ? "local_root" : "remote_root", e.target.value),
    }),
    "pairSideInput",
    side,
  );
  // ONE NODE, TWO SHAPES. The left row is a flex pair (field + `Choose…`); the right is a plain
  // block holding one field, and `display`/`gap` are both asserted — so a shared flex row fails the
  // frame on the side that has no button to sit beside.
  const row = fid(
    el(
      "div",
      { class: local ? "settings-pair-row" : "settings-pair-row-single" },
      input,
      local
        ? focusable(
            fid(
              inputButton(SETTINGS.choose, () => handlers.onChoose?.(), "11px 15px"),
              "pairChoose",
            ),
            "choose",
          )
        : null,
    ),
    "pairSideRow",
    side,
  );
  return fid(
    el(
      "div",
      { class: `settings-pair-side settings-pair-side-${local ? "local" : "remote"}` },
      fid(
        eyebrow({
          tone: local ? "up" : "down",
          text: local ? MAIN.sideLocal : MAIN.sideRemote,
          align: local ? "start" : "end",
        }),
        "pairSideLabel",
        side,
      ),
      row,
      fid(
        el(
          "div",
          { class: "settings-pair-note" },
          local ? SETTINGS.pairLocalNoteUnknown : SETTINGS.pairRemoteNote,
        ),
        "pairSideNote",
        side,
      ),
    ),
    "pairSide",
    side,
  );
}

/**
 * The live-updates panel — the one control on this screen that maps to its key with nothing lost.
 *
 * The key line reads `events_driven`, which is the engine's key; the frame draws
 * `event_driven_reconcile`, which does not exist. `14-behaviour-and-state.md:25` says so in as many
 * words, so this is the prototype being wrong rather than the app being unable — the node is left
 * unmapped rather than absorbed as a deviation, which is the call `5a Checking`'s unlit doors got.
 */
function livePanel(props) {
  const { config, handlers } = props;
  const on = config.events_driven !== false;
  const knob = toggle({
    on,
    label: SETTINGS.eventsDriven,
    onChange: (next) => handlers.onEvents?.(next),
  });
  focusable(knob, "events");
  fid(knob.querySelector(".toggle-knob"), "liveKnob");
  return fid(
    panel("settings-panel-row", [
      fid(
        el(
          "div",
          { class: "settings-panel-body" },
          fid(el("div", { class: "settings-panel-title" }, SETTINGS.eventsDriven), "liveTitle"),
          fid(el("div", { class: "settings-panel-sub" }, SETTINGS.eventsDrivenSub), "liveSub"),
          keyLine("events_driven"),
        ),
        "liveBody",
      ),
      fid(knob, "liveToggle"),
    ]),
    "livePanel",
  );
}

/**
 * The second cadence panel, on its Phase-1 subject.
 *
 * WHAT THE FRAME DRAWS HERE IS A SCHEDULE FOR THE FULL SWEEP, and there is no such thing to draw
 * (G4 #193). What the config does carry is `scan_interval_secs` — how often the daemon looks at
 * all, which is a different question with an honest answer, and the one §4 says to present in plain
 * language. So the shell, the head row and the divided control row are the frame's; the subject,
 * the title and what the row holds are Phase 1's.
 *
 * The head row keeps its shape with one child where the frame has two: the Weekly/Monthly control
 * is the schedule's, and there is no schedule.
 */
function timerPanel(props) {
  const { config, handlers } = props;
  const secs = config.scan_interval_secs ?? 300;
  return fid(
    panel("settings-panel-block", [
      fid(
        el(
          "div",
          { class: "settings-panel-head" },
          fid(
            el(
              "div",
              { class: "settings-panel-text" },
              fid(el("div", { class: "settings-panel-title" }, SETTINGS.timer), "timerTitle"),
              fid(el("div", { class: "settings-panel-sub" }, SETTINGS.timerSub), "timerSub"),
            ),
            "timerText",
          ),
        ),
        "timerHead",
      ),
      el(
        "div",
        { class: "settings-panel-control" },
        el("span", { class: "settings-control-label" }, SETTINGS.every),
        namedSteps(
          stepper({
            value: secs,
            format: intervalLabel,
            min: MIN_INTERVAL_SECS,
            max: MAX_INTERVAL_SECS,
            onStep: (delta) => handlers.onInterval?.(stepInterval(secs, delta)),
          }),
        ),
        el("span", { class: "settings-spacer" }),
        keyLine("scan_interval_secs"),
      ),
    ]),
    "timerPanel",
  );
}

/**
 * `Full sweep now` — `ControlCommand::Resync`, which latches the next pass to a full-tree walk and
 * schedules it. `sync_now` is NOT this: under the default config it runs an incremental pass, which
 * is the thing this panel offers an alternative to.
 *
 * Disabled while a pass is running, because asking for a sweep during one does nothing visible and
 * this project has already filed a button with no click feedback once (PR #140).
 */
function sweepPanel(props) {
  const { handlers, syncing } = props;
  const sweep = fid(
    button({
      kind: "primarySoft",
      label: SETTINGS.sweepNow,
      onClick: () => handlers.onSweep?.(),
      padding: "10px 20px",
      radius: "var(--r-10)",
      fontSize: "13px",
    }),
    "runButton",
  );
  focusable(sweep, "sweep");
  if (syncing) {
    setButtonKind(sweep, "primaryDisabled");
    sweep.disabled = true;
  }
  return fid(
    panel("settings-panel-row settings-panel-centred", [
      fid(
        el(
          "div",
          { class: "settings-panel-body" },
          fid(el("div", { class: "settings-panel-title" }, SETTINGS.fullSweep), "runTitle"),
          fid(el("div", { class: "settings-panel-sub" }, SETTINGS.fullSweepNoteUnknown), "runNote"),
        ),
        "runBody",
      ),
      sweep,
    ]),
    "runPanel",
  );
}

/** An eyebrow with the drawn spacing above it. `eyebrow()` takes no class, so it is added after. */
function sectionLabel(text, cls) {
  const node = eyebrow({ text });
  node.classList.add(cls);
  return node;
}

function foldersTab(props) {
  return [
    // The seam spans the two inputs and nothing else — `settingsPair` is 44px down and 86px tall,
    // sized explicitly rather than pinned to a block, because a full-width rule sits below it.
    fid(renderSeam({ site: "settingsPair" }), "seam"),
    fid(eyebrow({ text: SETTINGS.pairTitle }), "pairLabel"),
    fid(el("div", { class: "settings-pair" }, pairSide(props, 0), pairSide(props, 1)), "pairGrid"),
    fid(sectionLabel(SETTINGS.cadenceTitle, "settings-label-cadence"), "cadenceLabel"),
    livePanel(props),
    timerPanel(props),
    fid(sectionLabel(SETTINGS.runOne, "settings-label-run"), "runLabel"),
    sweepPanel(props),
  ];
}

// ------------------------------------------------------------------------ tab 2 · what to skip ----

function ruleRow(props, rule, i, pending) {
  // A rule added but not saved has nothing measured about it — the report was taken against the
  // config on disk. It says so rather than borrowing a neighbour's numbers or drawing zeros.
  const effect = pending ? { effect: SETTINGS.ruleNotSaved, detail: null, dim: false } : ruleEffect(rule);
  return fid(
    el(
      "div",
      { class: "settings-rule" + (effect.dim ? " is-stale" : "") },
      fid(el("span", { class: "settings-rule-pattern" }, rule.pattern), "rulePattern", i),
      fid(
        el(
          "div",
          { class: "settings-rule-body" },
          fid(el("div", { class: "settings-rule-effect" }, effect.effect), "ruleEffect", i),
          effect.detail
            ? fid(el("div", { class: "settings-rule-detail" }, effect.detail), "ruleDetail", i)
            : null,
        ),
        "ruleBody",
        i,
      ),
      focusable(
        fid(
          rowButton(SETTINGS.remove, () => props.handlers.onRemoveRule?.(rule.pattern)),
          "ruleRemove",
          i,
        ),
        `remove:${rule.pattern}`,
      ),
    ),
    "rule",
    i,
  );
}

/**
 * The rules list.
 *
 * EVERYTHING ABOVE THE FOOTER IS THE SAVED CONFIG; THE FOOTER IS THE STAGED EDIT. That reading is
 * the frame's own — `8a Skip rules` draws `video-raw/**` at full opacity while its footer says that
 * rule was removed, AND counts it inside `hiding 4 files, 3.1 GB in total`. Two independent signals
 * agree, and they have to: the counts were measured against the config on disk, so a list that
 * dropped the row while the total still counted it would be a screen disagreeing with itself. The
 * fixture recorded this as S6's call to make and it is made here.
 *
 * A STAGED ADDITION IS THE ONE EXCEPTION, and it is not really one: an added rule has no measured
 * row to leave alone, so it appears with `Not saved yet` where its counts would be. Without it,
 * `Add` would look like a control that does nothing — the removal at least turns the footer amber.
 */
function rulesBlock(props) {
  const { saved, config, skip, handlers, drafts } = props;
  const savedRules = saved?.exclude ?? [];
  const added = (config.exclude ?? []).filter((p) => !savedRules.includes(p));
  const rules = [...savedRules, ...added];
  const byPattern = new Map((skip?.rules ?? []).map((r) => [r.pattern, r]));
  const total = totalLine(skip);
  return fid(
    el(
      "div",
      { class: "settings-rules" },
      fid(
        el(
          "div",
          { class: "settings-rules-head" },
          fid(eyebrow({ text: SETTINGS.yourRules }), "rulesLabel"),
          fid(el("span", { class: "settings-spacer" }), "rulesSpacer"),
          // Omitted rather than zeroed while the walk is in flight: `hiding 0 files` is a claim, and
          // for the first second of every visit it would be the wrong one.
          total ? fid(el("span", { class: "settings-rules-total" }, total), "rulesTotal") : null,
        ),
        "rulesHead",
      ),
      // A NODE THE FRAME DOES NOT HAVE, and the one place this screen adds one. The window is fixed
      // at 764px and cannot grow, so a config with a dozen exclude rules pushes the add row and the
      // `.sync` note straight through the footer — `02-shell.md` calls that "a real bug found twice
      // during this design". The frames leave every node here `overflow: visible` and `overflow` is
      // an asserted property, so the scroll cannot go on any of them; it goes on a wrapper instead,
      // which is invisible to the gate because nothing stamps it. Rows keep their own keys.
      //
      // A CAP RATHER THAN A SCROLL WAS THE OTHER OPTION and it is the wrong one here. `+n more` is
      // right for the main screen's transfer rows, which are a report; these rows each carry the
      // only `Remove` button that rule will ever have, so hiding the twelfth would make it
      // unremovable from the screen that exists to remove it.
      el(
        "div",
        // Named so its position survives the ~2s rebuild — see `keepScroll` in app.js.
        { class: "settings-rules-scroll", "data-scroll": "skip-rules" },
        rules.map((pattern, i) =>
          ruleRow(props, byPattern.get(pattern) ?? { pattern }, i, !savedRules.includes(pattern)),
        ),
      ),
      fid(
        el(
          "div",
          { class: "settings-add" },
          fid(
            textInput({
              value: drafts?.exclude ?? "",
              placeholder: SETTINGS.addRulePlaceholder,
              mono: true,
              "data-field": "draft-exclude",
              "data-sfocus": "field:draft-exclude",
              "data-focus-key": '[data-sfocus="field:draft-exclude"]',
              onInput: (e) => handlers.onDraft?.("exclude", e.target.value),
              onKeydown: (e) => {
                if (e.key === "Enter") handlers.onAddRule?.();
              },
            }),
            "addInput",
          ),
          focusable(
            fid(
              inputButton(SETTINGS.add, () => handlers.onAddRule?.(), "11px 18px"),
              "addButton",
            ),
            "add:exclude",
          ),
        ),
        "addRow",
      ),
    ),
    "rules",
  );
}

function skipTab(props) {
  // `.sync` is set brighter inside its own sentence, which makes the note one node with an inline
  // child rather than two — `splitEmphasis` keeps the sentence whole so the copy gate still finds it.
  const [before, name, after] = splitEmphasis(SETTINGS.dotSyncNote, ".sync");
  // The daemon's standing "cannot be synced" list, already filtered to the local kinds by the one
  // function that decides that group (#232). The panel is drawn only when it has members: a
  // reassurance about a group with nothing in it is a sentence about nothing.
  const cannot = props.cannot ?? { count: 0, rows: [], kinds: "" };
  const dotSync = el(
    "div",
    { class: "settings-dotsync" },
    before,
    fid(el("span", { class: "settings-dotsync-name" }, name), "dotSyncName"),
    after,
  );
  // The 12px separates the note FROM THE PANEL, so it is set with the panel and not by the
  // stylesheet: with no panel above it the margin would be spacing this line against nothing.
  if (cannot.count > 0) dotSync.style.marginTop = "12px";
  return [
    fid(el("div", { class: "settings-skip-intro" }, SETTINGS.skipIntro), "skipIntro"),
    rulesBlock(props),
    fid(
      el(
        "div",
        { class: "settings-skip-tail" },
        cannot.count > 0
          ? fid(
              el(
                "div",
                { class: "settings-unsyncable" },
                fid(el("span", { class: "settings-unsyncable-glyph" }, "⊘"), "unsyncableGlyph"),
                fid(
                  el(
                    "div",
                    { class: "settings-unsyncable-note" },
                    SETTINGS.unsyncableNote(cannot.count, cannot.kinds),
                  ),
                  "unsyncableNote",
                ),
                fid(
                  button({
                    kind: "secondary",
                    label: SETTINGS.seeThem,
                    onClick: () => props.onSeeUnsyncable?.(),
                    padding: "7px 14px",
                    radius: "var(--r-8)",
                    fontSize: "12px",
                  }),
                  "seeThem",
                ),
              ),
              "unsyncable",
            )
          : null,
        fid(dotSync, "dotSyncNote"),
      ),
      "tail",
    ),
  ];
}

// ------------------------------------------------------------------------ tab 3 · deletions ----

const POLICY_COPY = {
  ask_every_time: { title: SETTINGS.askEvery, body: SETTINGS.askEverySub, note: SETTINGS.recommended },
  only_permanent: { title: SETTINGS.askPermanent, body: SETTINGS.askPermanentSub },
  never: { title: SETTINGS.askNever, body: SETTINGS.askNeverSub, tone: "destructive" },
};

function deletionsTab(props) {
  const { config, handlers, loaded } = props;
  // NO CARD UNTIL THE FILE HAS BEEN READ. `policyOf({})` answers `ask_every_time`, which is the
  // daemon's default and a true statement about an EMPTY config — and a lie about one that could
  // not be parsed, or one that simply has not come back yet. This is the screen's most consequential
  // control; it does not guess.
  const selected = loaded ? policyOf(config) : null;
  return [
    fid(el("div", { class: "settings-section-title" }, SETTINGS.deletionsTitle), "deletionsTitle"),
    fid(el("div", { class: "settings-section-sub" }, SETTINGS.deletionsSub), "deletionsSub"),
    fid(
      el(
        "div",
        { class: "settings-cards", role: "radiogroup", "aria-label": SETTINGS.deletionsTitle },
        POLICIES.map((policy, i) => {
          const copy = POLICY_COPY[policy.id];
          const card = fid(
            radioCard({
              selected: selected === policy.id,
              title: copy.title,
              note: copy.note ?? null,
              body: copy.body,
              tone: copy.tone ?? null,
              onSelect: () => handlers.onPolicy?.(policy),
            }),
            "card",
            i,
          );
          focusable(card, `policy:${policy.id}`);
          fid(card.querySelector(".radio-head"), "cardHead", i);
          fid(card.querySelector(".radio-ring"), "cardRing", i);
          fid(card.querySelector(".radio-title"), "cardTitle", i);
          fid(card.querySelector(".radio-text"), "cardBody", i);
          if (copy.note) fid(card.querySelector(".radio-note"), "cardBadge");
          return card;
        }),
      ),
      "cards",
    ),
    // Shipped as drawn. `deletion_policy` is not a key the daemon has — it is a name for the two
    // `[delete_approval]` booleans, and G5 (#194) would mint the real one. §68 made this call for
    // the label and the opposite one for `events_driven`, and the difference is which side is
    // wrong: there the frame names a key that does not exist where one does, here it names the
    // policy the pair expresses, which is what a person choosing between three cards is setting.
    keyLine("deletion_policy · applies to both directions"),
  ];
}

// -------------------------------------------------------------------- tab 4 · notifications ----

/** The dot beside each rule row. Three forms, and `11a Rules` draws the third one quiet. */
const RULE_DOTS = ["irreversible", "decision", "settled", "irreversible"];

/**
 * `11a Rules` — the reference sheet. No daemon data behind it: the content IS the policy, so it
 * renders from `NOTIFY.rules` and never changes.
 *
 * The Activity link is a real route change, not decoration — it is the door the sentence names.
 */
function rulesPanel(handlers) {
  const rows = NOTIFY.rules.interrupts.map((rule, i) =>
    fid(
      el(
        "div",
        { class: "notify-rule" },
        fid(el("span", { class: `notify-rule-dot is-${RULE_DOTS[i]}` }), "ruleDot", i),
        el(
          "div",
          { class: "notify-rule-text" },
          fid(el("div", { class: "notify-rule-title" }, rule.title), "ruleTitle", i),
          fid(el("div", { class: "notify-rule-why" }, rule.why), "ruleWhy", i),
        ),
      ),
      "rule",
      i,
    ),
  );
  rows.forEach((row, i) => fid(row.querySelector(".notify-rule-text"), "ruleBody", i));

  // A link is keyboard-reachable, so it needs a name like every other control on this screen: the
  // body is rebuilt on the ~2s poll and focus would otherwise land on `<body>` inside two ticks.
  const link = focusable(
    el(
      "a",
      {
        class: "notify-rules-link",
        href: "#",
        onClick: (e) => {
          e.preventDefault();
          handlers?.onRoute?.("activity");
        },
      },
      NOTIFY.rules.activityLink,
    ),
    "notify-activity",
  );

  return fid(
    el(
      "div",
      { class: "notify-rules" },
      fid(el("div", { class: "notify-rules-eyebrow" }, NOTIFY.rules.interruptsTitle), "interruptsTitle"),
      ...rows,
      fid(el("div", { class: "notify-rules-eyebrow is-silent" }, NOTIFY.rules.silentTitle), "silentTitle"),
      fid(
        el(
          "div",
          { class: "notify-silent" },
          ...NOTIFY.rules.silent.map((word, i) =>
            fid(el("span", { class: "notify-silent-chip" }, word), "silentChip", i),
          ),
        ),
        "silent",
      ),
      fid(
        el(
          "div",
          { class: "notify-rules-note" },
          NOTIFY.rules.activityBefore,
          fid(link, "activityLink"),
          NOTIFY.rules.activityAfter,
        ),
        "activityNote",
      ),
      fid(
        el(
          "div",
          { class: "notify-hard-rule" },
          fid(el("div", { class: "notify-hard-rule-title" }, NOTIFY.rules.hardRuleTitle), "hardRuleTitle"),
          fid(el("div", { class: "notify-hard-rule-body" }, NOTIFY.rules.hardRuleBody), "hardRuleBody"),
        ),
        "hardRule",
      ),
    ),
    "rulesRoot",
  );
}

/**
 * `11a Settings` — the `notify_policy` cards, the deletions tab's pattern at a different subject.
 *
 * No "until the file has been read" guard, and that asymmetry with `deletionsTab` is deliberate: the
 * deletion policy describes what the DAEMON does with files and must not be guessed at, while this
 * one is the GUI's own preference with a defined default. An unreadable `gui.toml` means the default
 * is in force, which is exactly what the first card says.
 */
function notifyPolicyColumn(props) {
  const { notifyPolicy, handlers } = props;
  const selected = notifyPolicy ?? NOTIFY_POLICIES[0].id;
  return fid(
    el(
      "div",
      { class: "notify-policy" },
      fid(el("div", { class: "settings-section-title" }, NOTIFY.settings.title), "policyTitle"),
      fid(el("div", { class: "settings-section-sub" }, NOTIFY.settings.sub), "policySub"),
      fid(
        el(
          "div",
          { class: "settings-cards", role: "radiogroup", "aria-label": NOTIFY.settings.title },
          NOTIFY_POLICIES.map((policy, i) => {
            const copy = NOTIFY.settings.choices[i];
            const card = fid(
              radioCard({
                selected: selected === policy.id,
                title: copy.label,
                note: i === 0 ? NOTIFY.settings.badge : null,
                body: copy.sub,
                onSelect: () => handlers?.onNotifyPolicy?.(policy.id),
              }),
              "card",
              i,
            );
            focusable(card, `notify:${policy.id}`);
            fid(card.querySelector(".radio-head"), "cardHead", i);
            fid(card.querySelector(".radio-ring"), "cardRing", i);
            fid(card.querySelector(".radio-title"), "cardTitle", i);
            fid(card.querySelector(".radio-text"), "cardBody", i);
            if (i === 0) fid(card.querySelector(".radio-note"), "cardBadge");
            return card;
          }),
        ),
        "cards",
      ),
      fid(keyLine(NOTIFY.settings.key), "policyKey"),
    ),
    "policyRoot",
  );
}

/**
 * The tab: the rules on the left, the choice on the right — which is what the first card's copy says.
 *
 * THE RULES PANEL SCROLLS, and it is the one thing here no gate could have caught. `11a Rules` is
 * 633px tall at the 600px it is drawn and neither crop is a window, so nothing in the harness
 * renders this tab at 1040×764. Measured instead: the window came out 974px against 764, a 210px
 * overflow that would have painted through the footer.
 *
 * A WRAPPER rather than `overflow` on the panel, for `.settings-rules-scroll`'s reason: the panel's
 * own box is what `11a Rules` describes, and a scroll declared on it would put a property of the
 * app's layout on the node the gate compares.
 */
function notificationsTab(props) {
  return [
    el(
      "div",
      { class: "settings-notify" },
      el("div", { class: "notify-rules-scroll", "data-scroll": "notify-rules" }, rulesPanel(props.handlers)),
      notifyPolicyColumn(props),
    ),
  ];
}

// ------------------------------------------------------------------------- tab 5 · advanced ----

/**
 * Not drawn anywhere. `08-settings.md` names six things this tab holds; two of them round-trip
 * through `ConfigUpdate` and four have no key and no command (G23 #237), so the tab says which four
 * rather than leaving someone to look for them.
 *
 * Same panel pattern as the cadence panels, because it is the same kind of thing: a plain-language
 * title, a sentence about what it does to your files, and the key underneath in mono.
 */
/**
 * One Advanced text field: title, sentence, mono key, input. The `proton_cli` panel above predates
 * this and is left as it is — it carries a placeholder note nothing else needs.
 *
 * `value ?? ""` and not `value || ""`: an empty string staged by clearing the field must draw as
 * cleared, not fall back to the saved value.
 */
function advancedField(key, title, sub, placeholder, { config, handlers }) {
  return panel("settings-panel-block", [
    panelText(title, sub),
    keyLine(key),
    el(
      "div",
      { class: "settings-panel-control" },
      textInput({
        value: config[key] ?? "",
        placeholder,
        mono: true,
        "data-field": key,
        "data-sfocus": `field:${key}`,
        "data-focus-key": `[data-sfocus="field:${key}"]`,
        "aria-label": title,
        onInput: (e) => handlers.onField?.(key, e.target.value),
      }),
    ),
  ]);
}

function advancedTab(props) {
  const { config, handlers, drafts } = props;
  const include = config.include ?? [];
  return [
    panel("settings-panel-block", [
      panelText(SETTINGS.includeTitle, SETTINGS.includeSub),
      keyLine("include"),
      el(
        "div",
        { class: "settings-panel-control settings-panel-list" },
        include.length === 0
          ? el("div", { class: "settings-rule-detail" }, SETTINGS.includeEmpty)
          : include.map((pattern) =>
              el(
                "div",
                { class: "settings-rule settings-rule-plain" },
                el("span", { class: "settings-rule-pattern" }, pattern),
                el("div", { class: "settings-rule-body" }),
                focusable(
                  rowButton(SETTINGS.remove, () => handlers.onRemoveInclude?.(pattern)),
                  `include:${pattern}`,
                ),
              ),
            ),
      ),
      el(
        "div",
        { class: "settings-add" },
        textInput({
          value: drafts?.include ?? "",
          placeholder: SETTINGS.addIncludePlaceholder,
          mono: true,
          "data-field": "draft-include",
          "data-sfocus": "field:draft-include",
          "data-focus-key": '[data-sfocus="field:draft-include"]',
          onInput: (e) => handlers.onDraft?.("include", e.target.value),
          onKeydown: (e) => {
            if (e.key === "Enter") handlers.onAddInclude?.();
          },
        }),
        focusable(
          inputButton(SETTINGS.add, () => handlers.onAddInclude?.(), "11px 18px"),
          "add:include",
        ),
      ),
    ]),
    panel("settings-panel-block", [
      panelText(SETTINGS.cliTitle, SETTINGS.cliSub),
      keyLine("proton_cli"),
      el(
        "div",
        { class: "settings-panel-control" },
        textInput({
          value: config.proton_cli ?? "",
          // THE DAEMON'S DEFAULT, AS A PLACEHOLDER. An absent key draws the field empty, which
          // reads as "nothing is set" when `proton-drive` is in fact what runs — the same
          // absent-is-not-empty confusion `ABSENT_DEFAULTS` fixes for the keys that DO draw their
          // default. A placeholder says it without staging anything. Not deck copy: it is a program
          // name, like the mono key lines §68 keeps out of `copy.js`.
          placeholder: "proton-drive",
          mono: true,
          "data-field": "proton_cli",
          "data-sfocus": "field:proton_cli",
          "data-focus-key": '[data-sfocus="field:proton_cli"]',
          "aria-label": SETTINGS.cliTitle,
          onInput: (e) => handlers.onField?.("proton_cli", e.target.value),
        }),
      ),
    ]),
    // THE THREE KEYS G23 (#237) ADDED, drawn in the same shape as the CLI field above because they
    // are the same kind of thing: a plain-language title, what changing it does, and the key in
    // mono. Each is a FILE key — an empty field clears it and the daemon default applies, which is
    // what `write_config` does with an empty string rather than writing `key = ""`.
    // THE DAEMON'S DEFAULTS, AS PLACEHOLDERS, and inline rather than in `copy.js` for the same
    // reason the `proton-drive` placeholder above is: they are config values, like the mono key
    // lines §68 keeps out of the deck. The socket's placeholder IS a sentence, so it stays copy.
    advancedField("log_level", SETTINGS.logTitle, SETTINGS.logSub, "info", props),
    advancedField("socket_path", SETTINGS.socketTitle, SETTINGS.socketSub, SETTINGS.socketPlaceholder, props),
    advancedField("conflict_suffix", SETTINGS.suffixTitle, SETTINGS.suffixSub, "proton-cloud", props),
    panel("settings-panel-block", [
      panelText(SETTINGS.configFileTitle, null),
      el(
        "div",
        { class: "settings-key settings-config-path" },
        config.exists === false ? SETTINGS.configFileMissing : (config.path ?? SETTINGS.configFileMissing),
      ),
    ]),
    el("div", { class: "settings-advanced-note" }, SETTINGS.advancedMissing),
  ];
}

// --------------------------------------------------------------------------------- the body ----

const TAB_BODIES = {
  folders: foldersTab,
  skip: skipTab,
  deletions: deletionsTab,
  notifications: notificationsTab,
  advanced: advancedTab,
};

/** The screen: a title block, the four pills, and whichever tab is showing. */
export function renderSettings(props = {}) {
  const tab = props.tab ?? "folders";
  const body = TAB_BODIES[tab] ?? TAB_BODIES.folders;
  // ABOVE EVERY TAB, when the file behind them could not be read. `read_config` rejects an
  // unparseable or unreadable config and the screen would otherwise draw it as an empty, valid one:
  // blank folders, live updates on, a timer at five minutes and a deletion policy card selected.
  // Every control below this line is describing a file nobody could open, so the line says so.
  const unreadable = props.configError
    ? el("div", { class: "settings-unreadable" }, SETTINGS.configUnreadable(props.configError))
    : null;
  const titleBlock = fid(
    el(
      "div",
      { class: "settings-title-block" },
      fid(el("div", { class: "settings-title" }, SETTINGS.title), "title"),
      fid(el("div", { class: "settings-sub" }, SETTINGS.sub), "sub"),
    ),
    "titleBlock",
  );
  const tabs = pillTabs({
    items: TABS,
    active: tab,
    onSelect: (id) => props.handlers?.onTab?.(id),
  });
  tabs.classList.add("settings-tabs");
  fid(tabs, "tabs");
  // BY THE TAB'S OWN ID, not by its position. The fifth pill sits fourth (Advanced is the technical
  // drawer and stays last), so a positional key would compare `Notifications` against the frame's
  // `Advanced` and report a width difference between two different words. `settingsFids`' `tab`
  // answers `undefined` for a tab no frame draws, and `fid` then stamps nothing.
  for (const [i, node] of [...tabs.children].entries())
    focusable(fid(node, "tab", TABS[i].id), `tab:${TABS[i].id}`);
  return [
    titleBlock,
    tabs,
    // `data-scroll` PER TAB, and the region scrolls at all because the doors are drawn under the
    // Save bar now (§94). This screen is rebuilt on every ~2s poll, so without the key app.js has
    // nothing to put the scroll position back on: measured at 29px back to 0 within three polls,
    // which makes the bottom of a tall tab unreadable. Per tab, because two tabs' contents have
    // nothing to do with each other.
    fid(
      el(
        "div",
        { class: `settings-content settings-content-${tab}`, "data-scroll": `tab:${tab}` },
        unreadable,
        body(props),
      ),
      "content",
    ),
  ];
}

/**
 * The footer action bar, which on this screen REPLACES the four doors.
 *
 * Its own builder rather than `renderActionBar`'s defaults because the note is one node in two
 * moods — the neutral saving promise, or the amber line naming what a staged change costs — and
 * because `Save` is built live and then disabled. A button born `primaryDisabled` attaches no
 * click listener (`button()` drops it), so arming it later would paint a live control that does
 * nothing; the same trap S4 records.
 */
export function renderSettingsBar(props = {}) {
  const { cost, dirty, saving, restartEnding, handlers } = props;
  // AMBER FOR EITHER WARNING. The cost line is one; so is "saving stops the sync that is running",
  // and so is a restart that left something unresolved — all three are a consequence rather than a
  // promise.
  const warned = Boolean(cost) || Boolean(restartEnding) || interrupts(props);
  const note = fid(
    el("span", { class: `bar-consequence settings-bar-note${warned ? " tone-cost" : ""}` }, barNoteOf(props)),
    "barNote",
  );
  const retry = barActionOf(props) === "restart";
  const discard = fid(
    button({
      kind: "secondary",
      label: retry ? SETTINGS.restart : SETTINGS.discard,
      onClick: () => (retry ? handlers?.onRestart?.() : handlers?.onDiscard?.()),
      padding: "11px 20px",
      radius: "var(--r-10)",
      fontSize: "13px",
    }),
    "discard",
  );
  const save = fid(
    button({
      kind: "primary",
      size: "bar",
      label: props.saveLabel ?? SETTINGS.save,
      onClick: () => handlers?.onSave?.(),
    }),
    "save",
  );
  if (!dirty || saving) {
    setButtonKind(save, "primaryDisabled");
    save.disabled = true;
  }
  const bar = el(
    "div",
    { class: "footer-action-bar settings-bar" },
    note,
    fid(el("span", { class: "shell-spacer" }), "barSpacer"),
    discard,
    save,
  );
  bar.dataset.shape = settingsBarShape(props);
  return fid(bar, "bar");
}

/**
 * Which action the bar's second slot carries: `discard`, or the retry after a restart that left
 * something wrong.
 *
 * THE SLOT IS `Discard changes` UNTIL A RESTART LEAVES SOMETHING WRONG. A save restarts the service
 * itself now (#320), so the settled-save state no longer needs an action of its own — but an ending
 * that leaves the file running ahead of the service ([`restartUnresolved`]) leaves the person with
 * nothing to press: `Save` is disabled by then, because nothing is staged. In two of those endings
 * the daemon is UP on the old settings, and this bar is then the only restart control left anywhere
 * in the app — the main screen and the tray offer a *start*, which a running daemon does not need.
 *
 * `configStaged`, NOT `dirty` (#335). The slot is only free while saving again would restart again,
 * and a staged **notification policy** is a `gui.toml` key the daemon has never heard of: that save
 * writes a file and restarts nothing, so yielding the retry to it would take the only way out away
 * and give nothing back. One predicate for "a daemon-config change is staged", read here, by
 * [`interrupts`] and by the save's own restart gate.
 *
 * Its own predicate rather than a line inside the builder: the tests here are pure (no DOM), so a
 * decision that only exists inside a `render` is a decision no test can reach.
 */
export function barActionOf({ restartEnding = null, configStaged = false } = {}) {
  return restartEnding && !configStaged ? "restart" : "discard";
}

/**
 * Is a save about to interrupt a pass? (#320/#335)
 *
 * Both halves, and neither alone: with nothing staged that the daemon reads there is nothing to
 * restart, and with no pass running there is nothing to interrupt.
 *
 * BOTH HALVES ARE NARROWER THAN THEY LOOK, and each was wrong in the same way — a predicate that
 * answered a neighbouring question:
 *
 * * `configStaged`, not `dirty`: `dirty` includes a staged notification policy, whose save writes
 *   `gui.toml` and restarts nothing. It drew this warning and then interrupted nothing.
 * * `countedSync`, not `syncing`: a plan-only rehearsal claims `syncing` — it must, or `activity`
 *   is gated off every status reply (`CLAUDE.md`) — so the Plan screen's own rehearsal made this
 *   name a sync that was not running. `app.js` tells them apart by the pass block's `kind`, which
 *   is the wire-visible half of `daemon.rs`'s `a_counted_pass_is_running`.
 */
const interrupts = ({ configStaged = false, countedSync = false } = {}) =>
  Boolean(configStaged) && Boolean(countedSync);

/** The endings `commands.rs`'s `RestartOutcome` names. Anything else is `unknown`. */
const RESTART_ENDINGS = new Set(["restarted", "not_running", "not_started", "never_stopped", "undetermined"]);

/**
 * The typed ending of a restart, from the command's Ok payload (#335).
 *
 * `unknown` for a tag this build has no sentence for — a backend newer than the window, or an
 * older one whose payload has no `ending` at all. Degrading here is the client half of the rule
 * `ListingOutcome`/`PlanOutcome` follow on the daemon's own wire: a client that told two endings
 * apart by matching a sentence is the bug #103 removes, and one that failed the whole reply over an
 * unrecognised tag would be no better.
 */
export function restartEndingOf(outcome) {
  const ending = outcome?.ending;
  return RESTART_ENDINGS.has(ending) ? ending : "unknown";
}

/**
 * Does this ending leave the file on disk running ahead of the service?
 *
 * THE ONE DEFINITION OF "there is still something to fix", read by every rule about how long the
 * state lives: the bar's retry slot, what survives navigating away, and what an edit forgets. Two
 * endings are settled — the service is running the new file, or it is deliberately not running —
 * and everything else has a way out that only a restart provides.
 */
export function restartUnresolved(ending) {
  return ending != null && ending !== "restarted" && ending !== "not_running";
}

/**
 * Does this daemon observation retire this ending? (#335)
 *
 * A latch describing a daemon state has to be re-validated against daemon state, or systemd's
 * `Restart=on-failure` brings the service up on the NEW settings while the bar still offers to
 * restart it. `app.js` already carries this exact rule one screen up, for `serviceStartError`.
 *
 * **The two failure endings invert, which is why this could not be written before they were typed.**
 * `not_started` was reached at a moment of *confirmed* absence — the socket was authoritatively
 * empty and the start then failed — so a daemon answering **later** is a process that began after
 * that moment and read the file this save wrote: the state is over, clear it. `never_stopped` is
 * the opposite: the daemon that is answering is the one that would not stop, still on the settings
 * it started with, so a reachable socket is the *problem* rather than the end of it. `undetermined`
 * observed nothing at all and may not conclude anything from a later poll either.
 *
 * **AND "LATER" IS THE WHOLE OF IT — evidence older than the outcome may never retire it.** The
 * review of #338 found this: the re-validation runs in `render()`, which reads the last *completed*
 * status poll, and `restartForSave` renders again the instant it records its answer. In the
 * `not_started` case that cached answer is *necessarily* a reachable daemon — the restart only took
 * the stop-then-start path **because** the probe said the daemon was running — so the latch was
 * nulled before it was ever drawn, and nothing re-latches it. The one sentence this whole issue
 * exists to show, and `Restart it now` with it, were unreachable in their own headline scenario.
 *
 * So the comparison is against the status **request** clock, not the reply's content: an answer may
 * speak only if its request was issued after the outcome was recorded (`store.beginStatus`). That
 * rules out the stale render *and* a poll that was already in flight when the restart finished —
 * which counting completed polls would not. It is a property of the data rather than of where
 * `render` is called from, so a future caller that renders cannot reintroduce it.
 *
 * `socketAnswers` is passed in rather than derived: `main.js`'s `clearsStartError` is already the
 * one definition of "the socket answers, by any route", and a second reading of the daemon state
 * here is how the two would drift.
 *
 * @param outcome  the latch — `{ ending, evidenceFloor }`
 * @param evidence what is known now — `{ socketAnswers, statusIssue }`
 */
export function clearsRestartFailure(outcome, evidence) {
  if (outcome?.ending !== "not_started" || !evidence?.socketAnswers) return false;
  return (evidence.statusIssue ?? 0) > (outcome.evidenceFloor ?? 0);
}

/**
 * The bar's sentence for one ending — **one per ending, and the ending is data** (#335).
 *
 * #328 had two typed endings and three collapsed into one `Err(String)`, so all three drew
 * `It is still running the old settings`: true of `never_stopped`, and the exact opposite of the
 * truth for `not_started`, where the stop succeeded and nothing is running at all. The sentence is
 * redistributed here rather than deleted.
 *
 * Exhaustive by ending with a `default` that claims nothing — a fall-through arm that reads as
 * "fine" is #246's shape, and this one is reachable by any backend this build does not know.
 */
export function saveNoteFor(ending, reason = "") {
  switch (ending) {
    case "restarted":
      return SETTINGS.savedRestarted;
    case "not_running":
      return SETTINGS.savedNotRunning;
    case "not_started":
      return SETTINGS.savedNothingRunning(reason);
    case "never_stopped":
      return SETTINGS.savedOldSettings(reason);
    case "undetermined":
      return SETTINGS.savedUnknownState(reason);
    default:
      return SETTINGS.savedUnknownEnding;
  }
}

/**
 * The bar's left-hand sentence: what just happened, what saving now would cost, the cost of a
 * staged change, the state a save left behind, or the standing promise about what saving writes —
 * in that order.
 */
export function barNoteOf(props = {}) {
  const { notice = null, cost = null, note = null } = props;
  // `notice` FIRST. It is the only one of these that is about something that just happened — a
  // sweep that did not start, a restart that failed — and everything below it is standing
  // information about a change that has not been made yet. Reporting the cost over the failure
  // would put the silence back one layer up.
  //
  // THE INTERRUPTION OUTRANKS THE COST LINE, and that is the one ordering #320 decides. Both are
  // standing information, but the cost line describes what a staged change would let through once
  // saved, while this describes what pressing `Save` does to a transfer that is happening now —
  // and the decision that made the restart automatic accepts the interruption only on condition
  // that it is never a surprise. A note it can hide would not satisfy that.
  return notice ?? (interrupts(props) ? SETTINGS.saveInterrupts : null) ?? cost ?? note ?? SETTINGS.saveNote;
}

/**
 * Everything the bar draws, as one string — so app.js can leave an unchanged bar alone without
 * diffing it. The NOTE ITSELF is in it, not just whether there is one: the cost line carries two
 * live numbers, and a shape that said only `cost` would freeze the first pair it was built with.
 */
export const settingsBarShape = (props = {}) =>
  [
    props.dirty ? "dirty" : "clean",
    props.saving ? "saving" : "idle",
    // The second slot's LABEL and handler, which is what this decides — `Discard changes` or the
    // retry an unresolved restart leaves behind (#320/#335). `barActionOf` and not `restartEnding`
    // alone: what decides the slot is the pair, and a shape that read one half would leave the bar
    // on screen with the wrong label when only the other moved.
    barActionOf(props),
    barNoteOf(props),
  ].join("|");

// ------------------------------------------------------------------------- the save refused ----

/**
 * `8a Save refused` — the dialog's contents. No title row and no ✕: it asks you to fix one thing,
 * and a dismiss in the corner would be a second answer to a question with one.
 *
 * TWO THINGS THE FRAME DRAWS THAT PHASE 1 CANNOT (G22 #236). `write_config` refuses on
 * `ConfigDoc::validate`, a serde/TOML check that never contacts Proton Drive — so it cannot say a
 * remote folder is missing, and `Create it on Proton Drive` has no command behind it. What is left
 * is the sentence `08-settings.md` calls the important one, which is true of every refusal: nothing
 * was saved, and the old settings are still running.
 */
function refusedMark() {
  const mark = fid(renderHexagon({ size: 34, state: "warning", flexNone: true }), "refusedMark");
  for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "refusedMarkPath", i);
  fid(mark.querySelector("circle"), "refusedMarkDot");
  return mark;
}

export function renderSaveRefused(props = {}) {
  const reason = refusalReason(props.error);
  return el(
    "div",
    { class: "settings-refused" },
    fid(
      el(
        "div",
        { class: "settings-refused-row" },
        refusedMark(),
        fid(
          el(
            "div",
            { class: "settings-refused-text" },
            fid(el("div", { class: "settings-refused-title" }, SETTINGS.refusedTitleUnknown), "refusedTitle"),
            fid(el("div", { class: "settings-refused-body" }, SETTINGS.refusedBodyUnknown), "refusedBody"),
            reason ? fid(el("div", { class: "settings-refused-reason" }, reason), "refusedReason") : null,
            fid(
              el(
                "div",
                { class: "settings-refused-actions" },
                fid(
                  button({
                    kind: "primarySoft",
                    label: SETTINGS.refusedBack,
                    onClick: () => props.onBack?.(),
                    padding: "9px 16px",
                    radius: "var(--r-9)",
                    fontSize: "12.5px",
                  }),
                  "refusedBack",
                ),
              ),
              "refusedActions",
            ),
          ),
          "refusedText",
        ),
      ),
      "refusedRow",
    ),
  );
}
