// The activity screen (S5) — what has happened, and where one file stands. `07-activity.md`.
//
// SIX FRAMES, TWO TABS AND THREE DIALOGS. `7a Activity quiet` and `6a Activity passes` are the same
// door wearing its two tabs; `7a File lookup` replaces the quiet body once a path resolves; `7a
// Never synced`, `6a Details` and `7a File pending` are dialogs over whichever of those is showing.
//
// THIS IS THE SCREEN THE DAEMON SAYS LEAST FOR, and the shape of the file follows from that. Every
// block below is either sourced or omitted — nothing is drawn from a plausible-looking substitute,
// because a number on this screen is a claim about whether someone's files are safe. What the
// frames draw and Phase 1 cannot answer, with the issue that closes each:
//
//   · the twenty-bar duration chart — NO per-pass duration exists anywhere. `StatusHistoryEntry` is
//     {epoch_secs, message, last_error, plan_summary, successful_sync_summary}; the sidecars carry
//     no timing either. The whole card is omitted (G12).
//   · `12,480 files · 41.2 GB` on both seam sides — no index-wide count (G7 #207). The numeral row
//     is omitted and the eyebrow keeps its sub-line, which IS sourced.
//   · `next full check in 4m` — counts down to a full-scan schedule that does not exist (G4 #193).
//     Deliberately NOT derived from `scan_interval`: `6a Details` draws that as 300s in the same
//     fixture, so the two would contradict each other and one of them would be false.
//   · the `Last things to move` rows — `status_history` is per-pass and carries no paths (G13).
//     The block's rows and head go; its footer row stays, because the way to the other tab is in it.
//   · `This file's history` — per-path history (G1 #190), and `07-activity.md` itself prescribes
//     omitting it. The `linked · id` line survives: `proton_id` is on the reply today.
//   · the pending dialog's progress bar — no fraction is computable in EITHER direction. An upload
//     has `bytes_total` and never `bytes_done`; a download has `bytes_done` and never `bytes_total`
//     (`daemon.rs`, `ipc.rs`). §63's rule applies: no track at all, never a bar reading zero.
//   · `Open folder` · `Open on Proton Drive` · `Open the system log` — no opener command exists and
//     no opener plugin is a dependency (G14). Omitted rather than painted dead: a live-looking
//     button that does nothing is the trap this project has already recorded twice (#224, #227).
//   · the never-synced `Can't be synced` group — a socket or a symlink never enters the index at
//     all, so there is nothing to enumerate (G15). Its GROUP goes; the rule-matched group stays,
//     and is the one block on this screen that a Phase-1 command gained rather than lost.
//
// THE ONE BLOCK THAT GOT A SOURCE. `skip_rule_usage` (C2) walks the local tree and reports, per
// exclude rule, the files it matches with their sizes. The F9 fixture recorded this group as
// unbuildable — "counting them means walking the filesystem, not reading the index" — which is
// exactly what C2 shipped, so the band and the dialog's first group are live data here.

import { el } from "../ui/el.js";
import { ACTIVITY, CHROME, CONFLICTS, MAIN } from "../ui/copy.js";
import { bytes, clock, count, dash, plural, since } from "../ui/format.js";
import { renderHexagon } from "../ui/hexagon.js";
import { renderSeam, seamMask } from "../ui/seam.js";
import { button, pillTabs } from "../ui/controls.js";
import { eyebrow, passRow, pathRow } from "../ui/rows.js";
import { noticeBand, warnGlyph } from "../ui/bands.js";
import { dialogBody, dialogFoot } from "../ui/dialog.js";
import { fid } from "../fixtures/frames.js";

/** The settled mark over the quiet seam and over a resolved lookup — both 52px, both masked. */
const SEAM_MARK = 52;
/** `7a File pending`'s one-way mark. 48px is in `strokeForSize`'s table for this frame alone. */
const PENDING_MARK = 48;

/**
 * The `Show them` / `Change this rule` / `Done` rung: a FILLED secondary at 9px radius.
 *
 * Written here rather than reached for from `bandButton`, which forces `kind:"decision"` — crimson
 * at 8px and weight 500. `Show them` sits in a band, so that builder is the reflex answer and the
 * wrong one: the frame measures `#101216` / `#23262D` / `#C9D0DA` at weight 400, and `07-activity`
 * names those three hexes in prose as well. A band about something merely absent does not get the
 * colour reserved for a decision.
 */
function filledSecondary(label, onClick, extra = {}) {
  return button({
    kind: "secondaryFilled",
    label,
    onClick,
    padding: "8px 15px",
    radius: "var(--r-9)",
    fontSize: "12.5px",
    ...extra,
  });
}

/** The `All 7 files` / `Sync passes` / `Change this rule` / `Open the system log` rung. */
function smallSecondary(label, onClick, { padding = "6px 13px", radius = "var(--r-8)" } = {}) {
  return button({ kind: "secondary", label, onClick, padding, radius, fontSize: "12px" });
}

// ------------------------------------------------------------------------------ the model ----

/**
 * Which footer rung this screen's current state draws — the second per-state answer in the app,
 * after S4's `footerKindOf`, and one field over from it.
 *
 * `7a Activity quiet` and `7a File lookup` are 18/14; `6a Activity passes` is the standard 18/15.
 * Same route, same four doors, one pixel apart — so the variant cannot live in `routes.js`, which
 * records a route's usual answer and has nowhere to put "except on the other tab".
 */
export function footerVariantOf(props) {
  return props?.tab === "passes" ? "standard" : "tight";
}

/**
 * What a looked-up path's reply means, as a verdict the screen can draw.
 *
 * FIVE OUTCOMES, ONE DRAWN. `path_sync_status` answers `synced`, `modified` or `conflict` for a
 * tracked path, reports `tracked:false` for anything the index has never seen, and distinguishes a
 * directory through `entity_kind`. The frames draw `synced` alone. The other four are reachable
 * from the first thing anyone types, which on this project is precisely where the bugs have been,
 * so each gets a verdict, a hexagon state and a deck sentence rather than a fallthrough.
 *
 * `mark` is `null` where no hexagon belongs: a miss is not a state a file is in.
 */
export function verdictOf(status, at = null) {
  if (!status || !status.tracked)
    return { key: "miss", mark: null, title: ACTIVITY.lookup.noMatch, sub: ACTIVITY.lookup.noMatchSub };
  if (status.entity_kind === "directory")
    return { key: "folder", mark: "outline", title: ACTIVITY.lookup.folder, sub: ACTIVITY.lookup.folderSub };
  switch (status.sync_status) {
    case "conflict":
      return {
        key: "conflict",
        mark: "needsDot",
        title: ACTIVITY.lookup.conflict,
        sub: ACTIVITY.lookup.conflictSub,
      };
    case "modified":
      return {
        key: "modified",
        mark: "outline",
        title: ACTIVITY.lookup.changed,
        sub: ACTIVITY.lookup.changedSub,
      };
    case "synced":
      return {
        key: "synced",
        mark: "settled",
        title: ACTIVITY.lookup.safe,
        // The `since` clause is a claim about WHEN, and `mtime` is the only timestamp on the reply.
        // With no mtime the sentence would be asserting a time it does not have, so it goes.
        sub: at ? ACTIVITY.lookup.safeSub(at) : null,
      };
    default:
      // An unrecognised `sync_status` is not "fine". `path_sync_status` documents three values and
      // the engine could grow a fourth; drawing the settled mark for a word this build has never
      // seen would say a file is safe on no evidence at all.
      return { key: "unknown", mark: "outline", title: ACTIVITY.lookup.changed, sub: null };
  }
}

/**
 * The rule-matched never-synced files, from `skip_rule_usage`.
 *
 * `unique_files` rather than `files`: a path matched by two rules is one file that is never synced,
 * and summing the per-rule counts would tell someone four files are missing when two are.
 */
export function neverSyncedFrom(report) {
  if (!report) return null;
  const rules = (report.rules ?? []).filter((r) => r.unique_files > 0);
  if (rules.length === 0) return null;
  return {
    total: rules.reduce((n, r) => n + r.unique_files, 0),
    rules,
  };
}

/**
 * The passes tab's summary sentence, composed from the history rather than asserted.
 *
 * `recovered` is the ordering fact the sentence depends on: "retried on its own" is true only
 * because a LATER pass succeeded. `entries` arrive newest-first, so the newest entry having no
 * error is the whole test.
 */
export function passesSummaryOf(entries) {
  const total = entries.length;
  if (total === 0) return null;
  const failed = entries.filter((e) => e.last_error != null).length;
  // NEWEST LAST on the wire — `daemon.rs` pushes each pass and drains from the front — so the
  // pass that decides whether the failures are behind us is the LAST entry, not the first. The
  // rows below reverse for display; this reads the array as it arrives.
  const recovered = entries[entries.length - 1]?.last_error == null;
  return ACTIVITY.passes.summary(total - failed, total, failed, recovered);
}

// ------------------------------------------------------------------- the shared page pieces ----

function titleBlock(sub, { paddingTop = "4px", subGap = "6px" } = {}) {
  const subNode = sub ? el("div", { class: "activity-sub" }, sub) : null;
  // 4px over 6px on the files tab, 6px over 7px on the passes tab. Two frames of the same screen
  // one pixel apart in both, which is measured rather than meaningful — but both are asserted.
  if (subNode) subNode.style.marginTop = subGap;
  const node = el(
    "div",
    { class: "activity-title-block" },
    fid(el("div", { class: "activity-title" }, ACTIVITY.title), "title"),
    subNode ? fid(subNode, "sub") : null,
  );
  node.style.paddingTop = paddingTop;
  return node;
}

/**
 * The lookup field. A REAL `<input>`, and that is a deliberate departure from the frame's tag.
 *
 * The prototype draws `span` + `span` + `span`, because a still image has nothing to type into. The
 * gate records a tag and never compares one — the four shipped screens already rely on it, since
 * every footer door is drawn as a `span` and emitted as a `<button>` — so an input styled to the
 * span's own numbers is measured identically and is the only form that can actually be used. The
 * alternative, a contenteditable dressed as a span, would trade a recorded tag for a worse control.
 */
function lookupField({ value, matches, onInput, onClear, inputRef }) {
  const input = el("input", {
    class: "activity-lookup-input",
    type: "text",
    value: value ?? "",
    placeholder: ACTIVITY.lookupPlaceholder,
    "aria-label": ACTIVITY.lookupPlaceholder,
    spellcheck: "false",
    autocomplete: "off",
  });
  input.addEventListener("input", (e) => onInput?.(e.target.value));
  if (inputRef) inputRef.node = input;

  const filled = Boolean(value);
  const field = el(
    "div",
    { class: `activity-lookup${filled ? " is-filled" : ""}` },
    fid(el("span", { class: "activity-lookup-icon" }, "⌕"), "searchIcon"),
    fid(input, "searchValue"),
    filled
      ? fid(el("span", { class: "activity-lookup-count" }, ACTIVITY.matches(matches ?? 0)), "searchCount")
      : fid(el("span", { class: "activity-lookup-hint" }, ACTIVITY.lookupShortcut), "searchHint"),
    filled
      ? fid(
          button({
            kind: "secondary",
            size: "icon",
            label: "✕",
            onClick: onClear,
            padding: "1px 6px",
            radius: "var(--r-6)",
            fontSize: "11px",
            class: "activity-lookup-clear",
            "aria-label": "Clear",
          }),
          "searchClear",
        )
      : null,
  );
  return fid(el("div", { class: "activity-lookup-wrap" }, field), "searchWrap");
}

/**
 * One side of a seam, counted.
 *
 * The numeral row the frames draw (`12,480` over `files · 41.2 GB`) is NOT built here: no Phase-1
 * command returns an index-wide count, `localTotals` is a value pinned for #207 and read by nothing
 * in `src/`, and rendering it would be inventing the one number on this screen that says how much
 * of someone's data is accounted for. The eyebrow and the sub-line below it are both sourced.
 */
function seamSide(which, sub) {
  const up = which === "local";
  return el(
    "div",
    { class: `activity-side activity-side-${which}` },
    eyebrow({
      tone: up ? "up" : "down",
      text: up ? MAIN.sideLocal : MAIN.sideRemote,
      align: up ? "start" : "end",
    }),
    sub ? el("div", { class: "activity-side-sub" }, sub) : null,
  );
}

// ------------------------------------------------------------------------------ files tab ----

/**
 * The never-synced band. Two nodes, not one — see `noticeBand`'s `wrapped`.
 *
 * The count is `unique_files` across the rules that matched anything. With the `can't be synced`
 * group unsourced the sentence's second clause drops, which is what `neverSyncedSub`'s zero form
 * is for: claiming `zero can't be synced at all` would be a sentence about a group nobody measured.
 */
function neverSyncedBand(never, onShow) {
  return fid(
    noticeBand({
      tone: "warn",
      wrapped: true,
      mark: fid(warnGlyph("⊘"), "bandGlyph"),
      title: fid(el("span", {}, ACTIVITY.neverSyncedTitle(never.total)), "bandTitle"),
      note: ACTIVITY.neverSyncedSub(never.total, 0),
      action: fid(filledSecondary(ACTIVITY.showThem, onShow, { class: "band-action" }), "bandAction"),
    }),
    "band",
  );
}

/**
 * The rung under the body — the sentence and the way to the other tab.
 *
 * WHAT IS LEFT OF `Last things to move`. Its head and its three rows both need per-file recent
 * activity, which nothing returns, so both go; this row stays because the tab switch is in it and
 * dropping it would strand the passes tab with no way in. `All 7 files` goes with the rows: it has
 * no destination in any frame, no id in `routes.js` and no count to put in it — the same shape as
 * S3's #224 and S4's #227, and this time there is nothing to file that the rows' own gap does not
 * already cover.
 */
function listFooter(onPasses) {
  return fid(
    el(
      "div",
      { class: "activity-list-foot" },
      fid(el("span", { class: "activity-list-note" }, ACTIVITY.quietIsNormal), "listNote"),
      el("span", { class: "activity-spacer" }),
      fid(smallSecondary(ACTIVITY.passesTab, onPasses), "passesButton"),
    ),
    "listFoot",
  );
}

function filesTab(props) {
  const { lookup, query } = props;
  // A resolved lookup REPLACES the title block and the quiet body. Not hidden and not pushed down:
  // `7a File lookup` has no title node at all, and the search field inherits the title block's
  // 4px padding-top and loses its own 18px margin.
  const looking = Boolean(lookup);

  const field = lookupField({
    value: query,
    matches: lookup?.status?.tracked ? 1 : 0,
    onInput: props.onQuery,
    onClear: props.onClearQuery,
    inputRef: props.inputRef,
  });
  // Promoted to first block, the field INHERITS the title block's 4px padding-top and gives up its
  // own 18px margin. Both halves matter: keeping the margin would push the whole screen down 18px.
  if (looking) {
    field.style.paddingTop = "4px";
    field.style.marginTop = "0";
  }

  return [
    looking ? null : titleBlock(props.quietSub),
    field,
    ...(looking ? lookupBody(props) : quietBody(props)),
  ];
}

function quietBody(props) {
  const { never, checkedAgo } = props;

  const mark = renderHexagon({ size: SEAM_MARK, state: "settled", masked: true });
  const agree = el("div", { class: "activity-agree" }, ACTIVITY.agree);
  // `position:false` matters: the frame records this label as STATIC. The mask's default writes
  // `position:relative`, which is an asserted property and would fail on the one node it is
  // applied to — the surrounding block already carries the stacking context.
  seamMask(agree, { pad: 16, position: false });

  const seamBlock = el(
    "div",
    { class: "activity-seam-block" },
    fid(renderSeam({ site: "activityQuiet" }), "seam"),
    el("div", { class: "activity-verdict" }, fid(mark, "hexagon"), fid(agree, "agree")),
    fid(
      el(
        "div",
        { class: "activity-sides" },
        fid(seamSide("local", checkedAgo ? ACTIVITY.watched(checkedAgo) : null), "sideLocal"),
        // NO right-hand sub-line. `next full check in 4m` counts down to a full-scan schedule the
        // daemon does not expose (G4 #193) — and deriving it from `scan_interval` instead would
        // contradict `6a Details`, which draws that same interval as its own row.
        fid(seamSide("remote", null), "sideRemote"),
      ),
      "sides",
    ),
  );

  const content = el(
    "div",
    { class: "activity-content" },
    never ? neverSyncedBand(never, props.onShowNeverSynced) : null,
    fid(el("div", { class: "activity-list" }, listFooter(props.onPasses)), "list"),
  );

  return [seamBlock, content];
}

// ------------------------------------------------------------------------- the lookup body ----

/** One side card: the size, and the absolute path that side keeps the file at. */
function lookupCard(which, { size, root, relative, meta }) {
  const up = which === "local";
  const abs = root ? `${root.replace(/\/$/, "")}/${relative}` : null;
  return el(
    "div",
    { class: `activity-card activity-card-${which}` },
    eyebrow({
      tone: up ? "up" : "down",
      text: up ? MAIN.sideLocal : MAIN.sideRemote,
      align: up ? "start" : "end",
    }),
    el(
      "div",
      { class: "activity-card-box" },
      el(
        "div",
        { class: "activity-card-meta" },
        el("span", {}, bytes(size)),
        // `edited HH:MM` only on the LOCAL side. `EmblemStatus.mtime` is `record.mtime`, the local
        // modification time; the reply carries no remote-side timestamp at all, so the frame's
        // `received 14:32` has nothing behind it (G16) and the clause is omitted rather than
        // filled with the local time wearing a remote label.
        meta ? el("span", {}, meta) : null,
      ),
      el("div", { class: "activity-card-path" }, abs ?? dash(null)),
    ),
  );
}

function lookupBody(props) {
  const { lookup, localRoot, remoteRoot } = props;
  const status = lookup.status;
  // The AGREED time, not the file's mtime. `Identical here and on Proton Drive since 14:32 today.`
  // names the pass that last verified the two sides — the frame pins it at the daemon's last sync,
  // one minute after the `edited 14:31` on the card below it. Using the mtime would put the file's
  // own edit where the check belongs and say the two sides matched before they did.
  const verdict = verdictOf(status, props.agreedAt);

  const hero = el("div", { class: "activity-hero" });
  if (verdict.mark) {
    hero.append(fid(renderHexagon({ size: SEAM_MARK, state: verdict.mark, masked: true }), "hexagon"));
  }
  const path = el("div", { class: "activity-hero-path" }, lookup.path);
  const title = el("div", { class: "activity-hero-title" }, verdict.title);
  seamMask(path, { pad: 14, position: false });
  seamMask(title, { pad: 16, position: false });
  hero.append(fid(path, "lookupPath"), fid(title, "lookupVerdict"));
  if (verdict.sub) {
    const sub = el("div", { class: "activity-hero-sub" }, verdict.sub);
    seamMask(sub, { pad: 14, padY: 2, position: false });
    hero.append(fid(sub, "lookupSub"));
  }

  const seamBlock = el(
    "div",
    { class: "activity-seam-block activity-seam-lookup" },
    fid(renderSeam({ site: "fileLookup" }), "seam"),
    hero,
  );

  // The two cards are drawn only for a path the index actually knows. A miss has no sides to
  // describe, and a folder has no single size.
  const carded = status?.tracked && status.entity_kind !== "directory";
  if (carded) {
    seamBlock.append(
      fid(
        el(
          "div",
          { class: "activity-cards" },
          lookupCard("local", {
            size: status.file_size,
            root: localRoot,
            relative: lookup.path,
            meta: props.editedAt
              ? `edited ${props.editedAt}`
              : status.mtime != null
                ? CONFLICTS.edited(status.mtime)
                : null,
          }),
          // A remote card claims a copy exists on Proton Drive, and `proton_id` is the only field
          // that proves one does. Without it the card would assert a second copy from nothing.
          status.proton_id
            ? lookupCard("remote", {
                size: status.file_size,
                root: remoteRoot,
                relative: lookup.path,
                meta: null,
              })
            : null,
        ),
        "cards",
      ),
    );
  }

  // WHAT IS LEFT OF `This file's history`. The four rows need per-path history (G1 #190) and
  // `07-activity.md` prescribes omitting them. The id line is the one thing in the block that is
  // sourced today, so it survives — in a rung of its own, because its `border-top` used to be the
  // rule closing the last history row and there is no last row now.
  const tail = status?.proton_id
    ? el(
        "div",
        { class: "activity-content" },
        fid(
          el(
            "div",
            { class: "activity-history-foot" },
            el("span", { class: "activity-spacer" }),
            fid(
              el("span", { class: "activity-linked" }, ACTIVITY.lookup.linked(elideId(status.proton_id))),
              "linked",
            ),
          ),
          "historyFoot",
        ),
      )
    : el("div", { class: "activity-content" });

  return [seamBlock, tail];
}

/**
 * `4c8f…9a21` — first four and last four of the NODE half of a composed id.
 *
 * `proton_id` is `volumeId~nodeId`; the volume half is the same for every file on the drive, so
 * eliding the whole string would show four characters that never change.
 */
export function elideId(protonId) {
  const node = String(protonId).split("~").pop() ?? "";
  return node.length <= 9 ? node : `${node.slice(0, 4)}…${node.slice(-4)}`;
}

// ----------------------------------------------------------------------------- passes tab ----

/**
 * One pass, as a row. `passRow` already matches the frame down to the failed variant's tinted
 * break-out and its quoted daemon string, so this is a mapping and not a builder.
 *
 * `detail` is the counters the pass reported; a pass with no summary has nothing to say about what
 * it moved, and an empty detail column is the truthful answer rather than `nothing to do`, which is
 * a claim that the pass ran and found nothing.
 */
function passRowFor(entry) {
  const failed = entry.last_error != null;
  return passRow({
    outcome: failed ? "failed" : "clean",
    label: failed ? ACTIVITY.passes.unreachable : ACTIVITY.passes.clean,
    detail: failed ? null : detailOf(entry),
    time: clock(entry.epoch_secs),
    error: failed ? entry.last_error : null,
  });
}

/** `2 sent, 1 brought here · 1 conflict kept` — the counters a pass reported, in the deck's terms. */
function detailOf(entry) {
  const s = entry.successful_sync_summary ?? entry.plan_summary;
  if (!s) return null;
  const moved = [];
  if (s.uploads) moved.push(`${count(s.uploads)} sent`);
  if (s.downloads) moved.push(`${count(s.downloads)} brought here`);
  const extra = [];
  if (s.conflicts) extra.push(`${count(s.conflicts)} ${plural(s.conflicts, "conflict", "conflicts")} kept`);
  if (moved.length === 0 && extra.length === 0) return ACTIVITY.passes.nothingToDo;
  return [moved.join(", "), extra.join(", ")].filter(Boolean).join(" · ");
}

function passesTab(props) {
  const { history } = props;

  // `pillTabs` IS the row, not something inside it. The frame puts the two pills, a flex spacer and
  // the `Details` button as direct children of `div[1]`, and `.pill-tabs` is already
  // `display:flex; gap:8px` — so nesting the builder inside a wrapper of my own would add a node
  // the frame does not have and shift every key below it.
  const tabs = pillTabs({
    items: [
      { id: "files", label: ACTIVITY.filesTab },
      { id: "passes", label: ACTIVITY.passesTab },
    ],
    active: "passes",
    onSelect: (id) => (id === "files" ? props.onFiles?.() : null),
  });
  tabs.classList.add("activity-tabs");
  tabs.append(
    el("span", { class: "activity-spacer" }),
    // The one body-level `Details` button in all 51 frames. It opens the same 522x462 overlay the
    // footer's fourth door does — the same route, reached twice on the one screen that has room
    // for it. Not a pill: 8px/13px padding against the tabs' 7px/15px.
    fid(
      button({
        kind: "secondary",
        label: CHROME.doors.details,
        onClick: props.onDetails,
        padding: "8px 13px",
        radius: "var(--r-9)",
        fontSize: "12.5px",
      }),
      "detailsButton",
    ),
  );
  fid(tabs, "tabs");

  // THE TWENTY-BAR CHART IS OMITTED, and it is the largest single thing this screen gives up. No
  // per-pass duration exists on the socket, in either sidecar, or anywhere in the engine's types —
  // there is nothing to degrade gracefully from. Drawing bars from any other number (a plan's
  // action count, say) would put a chart captioned `how long each took` over data about something
  // else entirely. G12.

  // NEWEST FIRST, which is the opposite of the wire. `6a Activity passes` draws 14:32 at the top
  // and 12:30 at the bottom; `status_history` arrives oldest-first. Reversed on a copy — the array
  // is the store's and `reverse()` mutates in place.
  const rows = [...history].reverse().map((entry, i) => fid(passRowFor(entry), "passRow", i));

  const foot = fid(
    el(
      "div",
      { class: "activity-passes-foot" },
      fid(el("span", { class: "activity-retention" }, ACTIVITY.passes.retention), "retention"),
      el("span", { class: "activity-spacer" }),
      // `Open the system log` omitted with the other two openers — G14.
    ),
    "passesFoot",
  );

  return [
    titleBlock(props.passesSub, { paddingTop: "6px", subGap: "7px" }),
    tabs,
    fid(el("div", { class: "activity-passes" }, ...rows, history.length ? foot : null), "passes"),
  ];
}

// ---------------------------------------------------------------------------------- render ----

export function renderActivity(props) {
  const nodes = props.tab === "passes" ? passesTab(props) : filesTab(props);
  return nodes.filter(Boolean);
}

// ------------------------------------------------------------------------------- dialogs ----

/**
 * `7a Never synced` — the rule-matched files, enumerated.
 *
 * ONE GROUP, NOT TWO. `Can't be synced` (a socket, a symlink) has no source in Phase 1 and cannot
 * acquire one by looking harder: those entries never enter the index, and `skip_rule_usage` reports
 * what the exclude rules matched, which is a different question. G15.
 */
export function renderNeverSyncedBody(props) {
  const never = props.never;
  const rules = never?.rules ?? [];
  const body = [];

  for (const [i, rule] of rules.entries()) {
    body.push(fid(eyebrow({ tone: "up", text: ACTIVITY.neverSyncedDialog.ruleHeading }), "ruleHeading", i));
    body.push(
      fid(
        el(
          "div",
          { class: "activity-rule-sub" },
          `${ACTIVITY.neverSyncedDialog.ruleSub("").trimEnd()} `,
          el("span", { class: "activity-rule-pattern" }, rule.pattern),
        ),
        "ruleSub",
        i,
      ),
    );
    // `samples` is capped at MAX_SAMPLES per rule by the walk, so a rule matching thousands of
    // files lists the first few and the count above says how many there are in total.
    for (const [j, sample] of (rule.samples ?? []).entries()) {
      body.push(fid(pathRow({ path: sample.path, note: bytes(sample.bytes), mono: true }), "ruleRow", i, j));
    }
    body.push(
      fid(
        smallSecondary(ACTIVITY.neverSyncedDialog.changeRule, props.onChangeRule, { padding: "7px 14px" }),
        "changeRule",
        i,
      ),
    );
  }

  return [
    dialogBody({ padding: "0 24px", marginTop: "20px", children: body }),
    dialogFoot({
      padding: "14px 24px 18px",
      marginTop: "14px",
      children: [
        fid(
          el("span", { class: "activity-reassurance" }, ACTIVITY.neverSyncedDialog.reassurance),
          "reassurance",
        ),
        el("span", { class: "activity-spacer" }),
        fid(filledSecondary(ACTIVITY.neverSyncedDialog.done, props.onClose), "done"),
      ],
    }),
  ];
}

/**
 * `6a Details` — the eight rows.
 *
 * DASH DOCTRINE THROUGHOUT, and this panel is where it matters most. Three of the eight live inside
 * a NULLABLE `last_plan_summary`, so a daemon that has not planned yet has no answer for them, and
 * a `0` there would be read as the truth about a sync rather than as the absence of one. Two more
 * (`scan_interval`, `event_stream`) come from `read_config`, which applies no defaults and can also
 * disagree with the live process — the daemon's own precedence is flag > file > default, and
 * `events_driven` degrades to snapshots at runtime when the keyring is unreadable. Default-filling
 * either would state something false about a running daemon; the file's silence is drawn as silence.
 */
export function renderDetailsBody(props) {
  const { counters, config, socketOk, historyCount } = props;
  const rows = [
    ["pending_changes", dash(counters.pending_changes)],
    ["conflicts", dash(counters.conflicts)],
    ["destructive_actions", dash(counters.destructive_actions)],
    ["skipped_unsupported", dash(counters.skipped_unsupported)],
    ["scan_interval", config?.scan_interval_secs != null ? `${config.scan_interval_secs}s` : dash(null)],
    ["event_stream", config?.events_driven == null ? dash(null) : config.events_driven ? "on" : "off"],
    ["source", historyCount > 0 ? "status_history" : dash(null)],
    ["socket", socketOk ? "connected" : "disconnected"],
  ];

  return [
    dialogBody({
      padding: "16px 22px 0",
      children: rows.map(([key, value], i) =>
        fid(
          el(
            "div",
            { class: "activity-kv" },
            fid(el("span", { class: "activity-kv-key" }, key), "kvKey", i),
            fid(
              el("span", { class: `activity-kv-value${key === "source" ? " is-quiet" : ""}` }, value),
              "kvValue",
              i,
            ),
          ),
          "kvRow",
          i,
        ),
      ),
    }),
    dialogFoot({
      padding: "14px 22px 16px",
      marginTop: "12px",
      children: [
        // `Copy all` stays. It needs no command — the clipboard is the webview's own — so it is the
        // one footer control on this screen that is not gated on a gap. `Open the system log` goes
        // with the other two openers (G14).
        fid(
          smallSecondary(ACTIVITY.copyAll, () => copyDetails(rows), { padding: "8px 14px" }),
          "copyAll",
        ),
      ],
    }),
  ];
}

/** The eight rows as `key value` lines — what the panel says, not the reply behind it. */
function copyDetails(rows) {
  const text = rows.map(([k, v]) => `${k} ${v}`).join("\n");
  navigator.clipboard?.writeText(text).catch((err) => console.error("copy failed:", err));
}

/**
 * `7a File pending` — one file, on its way.
 *
 * NO PROGRESS BAR. The frame draws a 3px track filled to exactly 41%, and no fraction is computable
 * in either direction: an upload carries `bytes_total` and never `bytes_done`, a download carries
 * `bytes_done` and never `bytes_total`. DEVIATIONS §63 already settled what to draw instead when
 * `2a Syncing` hit the same wall — no track at all, because a bar at zero reads as stalled — and
 * the dialog is shorter by the track and its margin for it.
 *
 * No head and no ✕ either: this dialog draws neither, so it takes no `dialogHead`. Esc still closes.
 */
export function renderFilePendingBody(props) {
  const t = props.transfer;
  const hero = el(
    "div",
    { class: "activity-pending-hero" },
    fid(renderHexagon({ size: PENDING_MARK, state: "syncing", direction: "up" }), "pendingHexagon"),
    fid(el("div", { class: "activity-pending-path" }, t.path), "pendingPath"),
    fid(el("div", { class: "activity-pending-title" }, ACTIVITY.lookup.pending), "pendingTitle"),
    t.bytes_total != null && t.started_epoch_secs != null
      ? fid(
          el(
            "div",
            { class: "activity-pending-sub" },
            ACTIVITY.lookup.pendingSub(since(t.started_epoch_secs), t.bytes_total),
          ),
          "pendingSub",
        )
      : null,
  );

  return [
    hero,
    fid(
      el(
        "div",
        { class: "activity-pending-foot" },
        fid(el("span", { class: "activity-pending-note" }, ACTIVITY.lookup.onlyLocal), "pendingNote"),
        el("span", { class: "activity-spacer" }),
        // `Open folder` omitted — G14.
      ),
      "pendingFoot",
    ),
  ];
}
