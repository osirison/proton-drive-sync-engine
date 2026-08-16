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
//     no timing either. The whole card is omitted (G16).
//   · `12,480 files · 41.2 GB` on both seam sides — no index-wide count (G7 #207). The numeral row
//     is omitted and the eyebrow keeps its sub-line, which IS sourced.
//   · `next full check in 4m` — counts down to a full-scan schedule that does not exist (G4 #193).
//     Deliberately NOT derived from `scan_interval`: `6a Details` draws that as 300s in the same
//     fixture, so the two would contradict each other and one of them would be false.
//   · the `Last things to move` rows — `status_history` is per-pass and carries no paths (G17).
//     The block's rows and head go; its footer row stays, because the way to the other tab is in it.
//   · `This file's history` — per-path history (G1 #190), and `07-activity.md` itself prescribes
//     omitting it. The `linked · id` line survives: `proton_id` is on the reply today.
//   · the pending dialog's progress bar — no fraction is computable in EITHER direction. An upload
//     has `bytes_total` and never `bytes_done`; a download has `bytes_done` and never `bytes_total`
//     (`daemon.rs`, `ipc.rs`). §63's rule applies: no track at all, never a bar reading zero.
//   · `Open folder` · `Open on Proton Drive` · `Open the system log` — WERE omitted for want of an
//     opener command (G18), and are drawn since #231, which added the four `open_*` commands. Two
//     carry a caveat rather than a gap. `Open on Proton Drive` opens the Drive web app and NOT the
//     file: a per-file link needs a share id and a link id, and nothing on the wire carries either
//     (`proton_id` is `volumeId~nodeId`, an API identity the web app does not route on).
//     `Open the system log` opens a snapshot of `journalctl --user -u proton-syncd`, because a
//     journal has no path to hand a file manager — and says so when there is no journal to read.
//   · `received 14:32` on the lookup's Proton card — WAS omitted for want of any remote-side
//     timestamp, and is drawn since #233 from `EmblemStatus.last_transfer`. Only its `up` direction
//     renders it; see `receivedAtFrom` for why, and for the four ordinary ways it is still absent.
//   · the never-synced `Can't be synced` group — WAS omitted because a socket or a symlink never
//     enters the index at all, and is drawn since #232. It is still not in the index: the daemon's
//     local walk now REPORTS what it drops, on `ControlResponse.unsyncable`.
//
// THE THREE BLOCKS THAT GOT A SOURCE. `skip_rule_usage` (C2) walks the local tree and reports, per
// exclude rule, the files it matches with their sizes — the F9 fixture recorded that group as
// unbuildable ("counting them means walking the filesystem, not reading the index"), which is
// exactly what C2 shipped. #232 and #233 are the other two, and neither came from looking harder at
// an existing field: each is a fact the engine did not record before.

import { el } from "../ui/el.js";
import { ACTIVITY, CHROME, CONFLICTS, MAIN } from "../ui/copy.js";
import { bytes, cardinal, clock, count, dash, plural, since } from "../ui/format.js";
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

/** The `All 7 files` / `Sync passes` / `Change this rule` rung. */
function smallSecondary(label, onClick, { padding = "6px 13px", radius = "var(--r-8)" } = {}) {
  return button({ kind: "secondary", label, onClick, padding, radius, fontSize: "12px" });
}

/**
 * The openers' own padding (#220/#231) — one pixel roomier than the rung above, in every frame that
 * draws one. Measured from `6a Activity passes`, `7a File lookup` and `7a File pending`, which all
 * put their opener at 31px tall; `6a Details` alone draws 33, and passes its own.
 */
const OPENER_PAD = { padding: "7px 14px" };

/**
 * WHY NOTHING OPENED, under the button that did not open it. No frame draws this — no frame draws a
 * failure — and without it the four openers fail the way they behaved before they were wired: a
 * click, and nothing. Machine text in mono, like every other quoted reason in this app (voice
 * rule 4). Returns `null` when there is nothing to say, so callers omit it by not appending.
 */
function openErrorLine(message) {
  return message ? el("div", { class: "activity-open-error" }, message) : null;
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
const VERDICTS = {
  // `sub` is a function throughout so the three entries have one shape, though only `synced` uses
  // the argument: its sentence names WHEN the two sides agreed, and with no time to put in it the
  // clause goes rather than asserting one the reply does not carry.
  synced: {
    mark: "settled",
    title: ACTIVITY.lookup.safe,
    sub: (at) => (at ? ACTIVITY.lookup.safeSub(at) : null),
  },
  modified: { mark: "outline", title: ACTIVITY.lookup.changed, sub: () => ACTIVITY.lookup.changedSub },
  conflict: { mark: "needsDot", title: ACTIVITY.lookup.conflict, sub: () => ACTIVITY.lookup.conflictSub },
};

export function verdictOf(status, at = null, error = null) {
  // A FAILED CHECK IS NOT A MISS. `path_sync_status` returning nothing because it could not open
  // the index looks identical to it returning nothing because the path is not there — and saying
  // "No file by that name in your sync folder" about a file sitting right in front of someone is
  // the worst answer this screen can give. The daemon's own words are carried through untouched
  // and quoted in mono at the call site (voice rule 4).
  if (error)
    return {
      key: "failed",
      mark: null,
      title: ACTIVITY.lookup.failed,
      sub: ACTIVITY.lookup.failedSub,
      error,
    };
  if (!status || !status.tracked)
    return { key: "miss", mark: null, title: ACTIVITY.lookup.noMatch, sub: ACTIVITY.lookup.noMatchSub };
  if (status.entity_kind === "directory")
    return { key: "folder", mark: "outline", title: ACTIVITY.lookup.folder, sub: ACTIVITY.lookup.folderSub };

  // An own-property check, NOT `VERDICTS[status.sync_status]` on its own. The status is a string off
  // the wire, so `constructor` and `__proto__` reach this lookup and answer with something from
  // Object's prototype — truthy, with no `mark` and no `sub`, so the call below would throw on a
  // reply the daemon could theoretically send. The switch this replaced could not be reached that
  // way; a table can, and that is the one thing a table costs.
  //
  // `hasOwnProperty.call` rather than `Object.hasOwn`: the latter is ES2022 and this file was its
  // only use in the app. `setup.sh` requires webkit2gtk-4.1, which implies a JSC that has it — so
  // this is not a fix for a known break, it is declining to raise the runtime floor for one line.
  const key = status.sync_status;
  const found = Object.prototype.hasOwnProperty.call(VERDICTS, key) ? VERDICTS[key] : null;
  // AN UNRECOGNISED STATUS IS A CHECK THAT CANNOT BE REPORTED, not a file that has changed. The
  // first version of this got the mark right and the words wrong: it withheld the settled hexagon
  // and then said "Changed here, not sent yet", which is a specific claim about a file whose state
  // this build cannot read. If the engine grows a fourth value that sentence is simply false, and a
  // false sentence about someone's file is what the whole screen is built to avoid. So it reports
  // the failure and quotes the value it did not understand.
  if (!found)
    return {
      key: "unknown",
      mark: null,
      title: ACTIVITY.lookup.failed,
      sub: ACTIVITY.lookup.unknownSub,
      error: `sync_status: ${JSON.stringify(key)}`,
    };
  return { key: status.sync_status, mark: found.mark, title: found.title, sub: found.sub(at) };
}

/**
 * The rule-matched never-synced files, from `skip_rule_usage`.
 *
 * `report.total_files` — "distinct files hidden by AT LEAST ONE rule" — and nothing derived from
 * the per-rule counts. Summing `files` double-counts a path two rules both hide; summing
 * `unique_files` does the opposite and DROPS it, because that field is `matches == 1` and exists
 * for a different question entirely (S6's `One rule removed — 4 files will start syncing`, where a
 * file another rule still hides would not start syncing). A machine whose every excluded file
 * matches two rules has `unique_files: 0` everywhere, and the band would vanish while the files
 * stayed unsynced.
 *
 * The band asks how many files are never synced. That is a union, and the report already computes
 * it on a distinct-file basis.
 */
export function neverSyncedFrom(report) {
  if (!report?.total_files) return null;
  // `files`, not `unique_files`, for WHICH rules to list: a rule that hides something belongs in
  // the dialog whether or not another rule hides it too.
  const rules = (report.rules ?? []).filter((r) => r.files > 0);
  if (rules.length === 0) return null;
  return { total: report.total_files, rules };
}

/**
 * The `Can't be synced` group, from `ControlResponse.unsyncable` (#232).
 *
 * THE ONE PLACE THE GROUP IS DECIDED. The band's count, the band's second clause, the dialog's
 * title and the dialog's rows all read this — four numbers that must be the same number, and this
 * screen has already shipped the bug where two of them were computed apart.
 *
 * Membership and the row notes come from ONE table, `ACTIVITY.neverSyncedDialog.cannotKind`, which
 * lives under the sentence it has to be true of. A reason with no entry there is still drawn, with
 * its raw token as the note — the rule `proton-sync status` follows for an unfamiliar reason, and
 * the conservative direction: a newer daemon's local kind must not vanish from the one dialog that
 * names what cannot sync. The single exclusion is `remote_not_downloadable`, which is a real file
 * on Proton Drive rather than a non-file in your folder, so both of this dialog's sentences would
 * be false about it.
 *
 * Ordered by path, so the list does not reshuffle between polls; the daemon already sorts, and
 * this does not depend on it.
 */
const CANNOT_SYNC_EXCLUDED = "remote_not_downloadable";

export function cannotSyncFrom(unsyncable) {
  const rows = (unsyncable ?? [])
    .map((item) => ({ ...item, path: pathOfUnsyncable(item) }))
    .filter((item) => item.path != null && item.reason !== CANNOT_SYNC_EXCLUDED)
    .map((item) => ({
      path: item.path,
      reason: item.reason,
      note: nounFor(item.reason).one,
    }))
    .sort((a, b) => a.path.localeCompare(b.path));
  return { count: rows.length, rows, kinds: kindsPhrase(rows) };
}

/**
 * TWO WIRE SHAPES, ONE GROUP — the engine spells this path differently on its two carriers, and
 * both reach this function (#315).
 *
 * `ipc::UnsyncableItem.path` is the daemon's persistent merged store on `ControlResponse.unsyncable`
 * (S5's dialog, S6's skip tab). `index::UnsyncableEntry.relative_path` is one walk's observation on
 * `DryRunReport.cannot_sync` (S7's review step, which runs before any daemon exists and so has no
 * store to read). They are deliberately different types — one has an age and a merge rule, the other
 * has neither — and reading whichever is present is the same accommodation `transfersOf` makes for
 * `transfers` / `transfer`. It is NOT a merge: one item carries one of the two, never both.
 *
 * Reading only `path` would silently drop every row of a `cannot_sync` list and render an empty
 * group with no error anywhere — the "block that renders nothing passes" shape, which this codebase
 * has shipped before and now has a gate for.
 */
function pathOfUnsyncable(item) {
  return item?.path ?? item?.relative_path ?? null;
}

/**
 * The noun pair for one reason token, or the raw token standing in for both.
 *
 * An unfamiliar token is drawn rather than hidden (see `cannotKind`), and it has to survive being
 * counted as well as being labelled — `two local_doodads` is ugly and true, where dropping the row
 * is neither.
 */
function nounFor(reason) {
  return ACTIVITY.neverSyncedDialog.cannotKind[reason] ?? { one: reason, many: reason };
}

/**
 * `a socket and two shortcuts` — the kinds present, counted, in the deck's own order.
 *
 * TWO SCREENS DRAW THIS ONE CLAUSE AT TWO MULTIPLICITIES, which is what settled its shape: S6's
 * skip tab draws `Two more files can't be synced no matter what — a socket and a shortcut` (one of
 * each), and S7's review step draws `3 files can't be synced — a socket and two shortcuts` (one
 * socket, two symlinks). An uncounted list of distinct kinds — what this was until #315 — renders
 * the second as `a socket and a shortcut`, naming two things beside a number that says three.
 *
 * So each kind carries its own count: `one` at one, `cardinal(n, "mid")` plus `many` above it. The
 * count in the sentence beside this stays the FILE count and this stays the answer to "what sort of
 * things are these" — the two agree by construction now instead of by coincidence.
 *
 * ORDERED BY `cannotKind`, NOT BY THE ROWS. It was first-appearance order over a path-sorted list,
 * which is a fact about filenames: `9a Review`'s own three sort `Desktop/inbox.lnk`,
 * `Desktop/proton.desktop`, `run/daemon.sock` and render `two shortcuts and a socket` — the drawn
 * sentence backwards, because of where a socket happens to live. Renaming a file would reorder a
 * sentence about kinds. The deck's table is an editorial order (socket, shortcut, pipe, device, and
 * the two residuals last) and it reads the same whatever the folder holds. A token the table does
 * not know keeps first-appearance order after all of them — there is nowhere else to put it.
 *
 * (`cardinal` hands back to digits above ten, so a folder with twelve sockets reads `12 sockets`
 * mid-sentence rather than spelling a number the deck spells nowhere.)
 */
function kindsPhrase(rows) {
  const counted = new Map();
  for (const row of rows) counted.set(row.reason, (counted.get(row.reason) ?? 0) + 1);
  const deck = Object.keys(ACTIVITY.neverSyncedDialog.cannotKind);
  const rank = (reason) => {
    const at = deck.indexOf(reason);
    return at < 0 ? deck.length : at;
  };
  const kinds = [...counted]
    .sort(([a], [b]) => rank(a) - rank(b))
    .map(([reason, n]) => {
      const noun = nounFor(reason);
      return n === 1 ? noun.one : `${cardinal(n, "mid")} ${noun.many}`;
    });
  if (kinds.length === 0) return "";
  if (kinds.length === 1) return kinds[0];
  return `${kinds.slice(0, -1).join(", ")} and ${kinds[kinds.length - 1]}`;
}

/**
 * The passes tab's summary sentence, composed from the history rather than asserted.
 *
 * `recovered` is the ordering fact the sentence depends on: "retried on its own" is true only
 * because a LATER pass succeeded. `entries` arrive OLDEST-FIRST — `daemon.rs` pushes each pass and
 * drains the front — so the newest is the LAST one, and its having no error is the whole test.
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
 * The lookup field. A `contenteditable` SPAN, and the tag is not a free choice here.
 *
 * The prototype draws `span` + `span` + `span`, because a still image has nothing to type into, and
 * the obvious answer is an `<input>` styled to the span's numbers — the gate records a tag and never
 * compares one, which is why every footer door is drawn as a `span` and emitted as a `<button>`.
 *
 * IT DOES NOT WORK, and the reason is worth writing down so nobody tries it again. Chromium's user-
 * agent stylesheet sets `overflow: clip !important` on text inputs, and a UA `!important` outranks
 * an author `!important` in the cascade — so an `<input>` computes `overflow: clip` and the drawn
 * span computes `visible`, forever, whatever the author writes. Verified rather than assumed. That
 * is an ASSERTED property, so the tag choice leaks out of "recorded but never compared" into a real
 * mismatch on this screen's primary control.
 *
 * The two alternatives were worse. Excluding `overflow` from the gate would absorb an app decision
 * into the harness — the width/height exclusions are for things NO app choice controls, which this
 * is not — and would quietly stop asserting it on every future input. Unmapping the node would
 * trade one property for all of them: colour, both font states, and the flex sizing that makes the
 * field's two states differ.
 *
 * `contenteditable="plaintext-only"` is WebKit's own (and WebKitGTK is the runtime), keeps the
 * drawn tag, and types. Its costs are handled at the three places below: the placeholder is a
 * `::before` rather than text, Enter is swallowed, and newlines are stripped from a paste.
 */
function lookupField({ value, matches, onInput, onClear, inputRef }) {
  const filled = Boolean(value);
  const input = el(
    "span",
    {
      // `is-empty` COMPUTED FROM THE VALUE, never from `:empty`. WebKit leaves a stray `<br>` in a
      // contenteditable the user has cleared, so `:empty` stops matching the moment someone types
      // and deletes — the placeholder would vanish for good. The body re-renders per keystroke,
      // which also normalises the `<br>` away.
      class: `activity-lookup-input${filled ? "" : " is-empty"}`,
      contenteditable: "plaintext-only",
      role: "searchbox",
      "aria-label": ACTIVITY.lookupPlaceholder,
      "data-placeholder": ACTIVITY.lookupPlaceholder,
      spellcheck: "false",
    },
    value ?? "",
  );
  // `plaintext-only` still inserts a newline on Enter and still accepts one from a paste, and a
  // path with a newline in it is not a path. Both are stopped here rather than in the handler.
  input.addEventListener("keydown", (e) => {
    if (e.key === "Enter") e.preventDefault();
  });
  input.addEventListener("input", () => onInput?.(input.textContent.replace(/[\r\n]+/g, "")));
  if (inputRef) inputRef.node = input;

  const field = el(
    "div",
    { class: `activity-lookup${filled ? " is-filled" : ""}` },
    fid(el("span", { class: "activity-lookup-icon" }, "⌕"), "searchIcon"),
    fid(input, "searchValue"),
    // `matches: null` means ASKED-BUT-NOT-ANSWERED, and it renders as nothing rather than as a
    // count. The lookup is debounced, so for 180ms after a keystroke the resolved answer belongs to
    // an older query — and rendering `0 matches` there flashes a false negative at exactly the
    // moment someone is typing a path that does match. The empty span keeps the node the frame has.
    filled
      ? fid(
          el("span", { class: "activity-lookup-count" }, matches == null ? "" : ACTIVITY.matches(matches)),
          "searchCount",
        )
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
  fid(field, "search");
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
 * The two groups as one band/dialog subject: how many files are never synced, and how that splits.
 *
 * `byRule` is `report.total_files` — the DISTINCT-FILE UNION, see `neverSyncedFrom`, which is the
 * one field that answers it. Not `unique_files`: that is `matches == 1` and belongs to a different
 * question. This comment said otherwise for two commits after the code was corrected, which is
 * precisely how someone "fixes" the code back.
 *
 * The two halves are independent: a machine with a socket and no exclude rules has `byRule: 0` and
 * is still a machine with a file that will never be copied anywhere, so `null` is returned only
 * when BOTH are empty. Before #232 the rule half alone decided, and that machine drew nothing.
 */
export function neverSyncedSubject(report, unsyncable) {
  const never = neverSyncedFrom(report);
  const cannot = cannotSyncFrom(unsyncable);
  const byRule = never?.total ?? 0;
  if (byRule === 0 && cannot.count === 0) return null;
  return { total: byRule + cannot.count, byRule, cannot, rules: never?.rules ?? [] };
}

/**
 * The never-synced band. Two nodes, not one — see `noticeBand`'s `wrapped`.
 *
 * Both clauses of the sentence are sourced now (#232). `neverSyncedSub`'s zero form still matters
 * and is still reachable — a machine whose only never-synced files match a rule renders one clause,
 * because claiming `zero can't be synced at all` would be a sentence about a group with no members.
 */
function neverSyncedBand(never, onShow) {
  const band = noticeBand({
    tone: "warn",
    wrapped: true,
    mark: fid(warnGlyph("⊘"), "bandGlyph"),
    // A PLAIN STRING, not a node. Wrapping it in a span to have something to stamp puts the fid on
    // an inline child of the title div — so the gate compared the frame's 806px block against a
    // 146px inline span. `noticeBand` builds the real title node; it is found afterwards.
    title: ACTIVITY.neverSyncedTitle(never.total),
    note: ACTIVITY.neverSyncedSub(never.byRule, never.cannot.count),
    action: fid(filledSecondary(ACTIVITY.showThem, onShow, { class: "band-action" }), "bandAction"),
  });
  fid(band.querySelector(".band-notice-title"), "bandTitle");
  fid(band.querySelector(".band-notice-note"), "bandNote");
  fid(band.querySelector(".band-notice-body"), "bandBody");
  fid(band.querySelector(".band-notice"), "bandRow");
  return fid(band, "band");
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
 *
 * AND `ACTIVITY.nothingRecent` IS NOT DRAWN HERE, which is worth stating because it looks like it
 * should be. `14-behaviour-and-state.md:129` gives it as the empty state for `Activity › files`:
 * "Nothing has moved in the last hour." But that sentence is a CLAIM about the last hour, and the
 * gap that removed the rows (#230) is exactly the absence of any per-file record to make it from —
 * so the app cannot know whether it is true. `quietIsNormal` is the frame's own sentence, says
 * nothing the daemon has not reported, and is what stays.
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
  const { lookup, query, matches } = props;
  // A resolved lookup REPLACES the title block and the quiet body, and so does the chooser. Not
  // hidden and not pushed down: `7a File lookup` has no title node at all, and the search field
  // inherits the title block's 4px padding-top and loses its own 18px margin.
  const choosing = !lookup && (matches?.matches?.length ?? 0) > 1;
  const looking = Boolean(lookup) || choosing;

  // Only the count for the string that is IN the field. Against what was TYPED, not the resolved
  // query: the two differ for a pasted absolute path, and `3 matches` beside a different string is
  // a count for a search nobody is looking at. Debounced, so this is false for 180ms after a
  // keystroke — deliberately, since `0 matches` there flashes a false negative mid-path.
  const asked = normaliseQuery(query);
  const answered = matches && matches.typed === asked;
  const field = lookupField({
    value: query,
    matches: answered ? matches.total : null,
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
    ...(choosing ? chooserBody(props) : lookup ? lookupBody(props) : quietBody(props)),
  ];
}

// --------------------------------------------------------------------- the chooser body ----

/**
 * One row of the chooser: the path the index stores, and where that file stands.
 *
 * `data-match-path` is how app.js puts the keyboard back on this row after the ~2s poll rebuilds
 * the screen. By path and not by index — a landing reply can reorder the list.
 */
function matchRow(match, onChoose) {
  const verdict = verdictOf(match.status);
  return el(
    "button",
    { class: "activity-match", "data-match-path": match.path, onClick: () => onChoose?.(match) },
    el("span", { class: "activity-match-path" }, match.path),
    el("span", { class: "activity-match-verdict" }, verdict.title),
  );
}

/**
 * Several files matched, so the screen asks instead of answering.
 *
 * NO HEXAGON AND NO SEAM. Both are claims about one file's standing, and there is no one file yet —
 * the mark over a list would be a verdict about whichever row it happened to sit above.
 */
function chooserBody(props) {
  const { matches } = props;
  const shown = matches.matches;
  const list = el(
    "div",
    { class: "activity-matches" },
    el("div", { class: "activity-matches-title" }, ACTIVITY.lookup.chooseTitle),
    el("div", { class: "activity-matches-sub" }, ACTIVITY.lookup.chooseSub),
    el(
      "div",
      { class: "activity-matches-list" },
      ...shown.map((match) => matchRow(match, props.onChooseMatch)),
    ),
    // Said only when it is true: a capped list that claims to be everything is how someone concludes
    // their file is not there.
    matches.total > shown.length
      ? el("div", { class: "activity-matches-capped" }, ACTIVITY.lookup.capped(shown.length, matches.total))
      : null,
  );
  return [list];
}

function quietBody(props) {
  const { never, checkedAgo } = props;

  // THE VERDICT IS DRAWN ONLY WHEN IT IS KNOWN TO BE TRUE. `Both sides agree` over a settled
  // hexagon is the strongest claim in the app, and it was rendered unconditionally here — on a
  // paused daemon, a syncing one, an unreachable one and a first run alike. `7a Activity quiet`
  // draws the idle case because that is the frame; the screen runs in all of them.
  //
  // Omitted rather than replaced: no frame draws this screen in any other state and the deck has no
  // sentence for one, so inventing a verdict would be the second mistake. The seam and both sides
  // stay — their sub-lines are already gated on having something true to say.
  const verdict = props.agreed
    ? (() => {
        const mark = renderHexagon({ size: SEAM_MARK, state: "settled", masked: true });
        for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "hexPath", i);
        const agree = el("div", { class: "activity-agree" }, ACTIVITY.agree);
        // `position:false` matters: the frame records this label as STATIC. The mask's default
        // writes `position:relative`, an asserted property, and would fail on the one node it is
        // applied to — the surrounding block already carries the stacking context.
        seamMask(agree, { pad: 16, position: false });
        return fid(
          el("div", { class: "activity-verdict" }, fid(mark, "hexagon"), fid(agree, "agree")),
          "verdict",
        );
      })()
    : null;

  const seamBlock = el(
    "div",
    { class: "activity-seam-block" },
    fid(renderSeam({ site: "activityQuiet" }), "seam"),
    verdict,
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

  return [fid(seamBlock, "seamBlock"), fid(content, "content")];
}

// ------------------------------------------------------------------------- the lookup body ----

/**
 * The Proton-side card's `received HH:MM`, or `null` when nothing sources it (#233).
 *
 * ONLY AN UPLOAD SAYS "RECEIVED". `EmblemStatus.last_transfer` is when this engine last moved the
 * file's bytes and which way, off the daemon's history log — and only `up` means Proton Drive
 * received them. A `down` row says when THIS COMPUTER received them, and a conflict sidecar's fetch
 * is a `down` row filed under the file's own path, so labelling either as a remote event would tell
 * someone the opposite of what happened on the one screen whose job is saying where a file stands.
 *
 * `null` is the ordinary answer, not an error: nothing ever transferred, the last transfer aged out
 * of the log's 90-day retention, the file was adopted rather than transferred, or it has moved
 * since. The clause is omitted. There is deliberately NO fallback to `mtime` — that is the local
 * modification time, and wearing a remote label is precisely what this card refused to ship.
 *
 * `pinned` is the frame's clock literal and is read only once the field is there, so a fixture can
 * never conjure the clause out of a status that has no transfer (the `clock.js` rule: an absolute
 * time moves with the machine's timezone, so a drawn one is pinned beside the epoch).
 */
export function receivedAtFrom(status, pinned = null) {
  const transfer = status?.last_transfer;
  if (!transfer || transfer.direction !== "up") return null;
  return pinned ?? clock(transfer.epoch_secs);
}

/** One side card: the size, and the absolute path that side keeps the file at. */
function lookupCard(which, { size, root, relative, meta }) {
  const up = which === "local";
  const abs = root ? `${root.replace(/\/$/, "")}/${relative}` : null;
  // The side's own index, which is what the fixture keys every slot below on — `local` is drawn
  // first in both frames. Positional, not a lookup, because the caller draws the pair in order.
  const s = up ? 0 : 1;
  const sizeSpan = fid(el("span", {}, bytes(size)), "cardSize", s);
  const metaRow = fid(
    el(
      "div",
      { class: "activity-card-meta" },
      sizeSpan,
      // `edited HH:MM` on the LOCAL side from `EmblemStatus.mtime`, `received HH:MM` on the Proton
      // side from `EmblemStatus.last_transfer` (#233). Two different fields for two different
      // events — see `receivedAtFrom` for why the remote clause can never be filled from the
      // local mtime, and why it is often absent.
      meta ? el("span", {}, meta) : null,
    ),
    "cardMeta",
    s,
  );
  return fid(
    el(
      "div",
      { class: `activity-card activity-card-${which}` },
      fid(
        eyebrow({
          tone: up ? "up" : "down",
          text: up ? MAIN.sideLocal : MAIN.sideRemote,
          align: up ? "start" : "end",
        }),
        "cardLabel",
        s,
      ),
      fid(
        el(
          "div",
          { class: "activity-card-box" },
          metaRow,
          fid(el("div", { class: "activity-card-path" }, abs ?? dash(null)), "cardPath", s),
        ),
        "cardBox",
        s,
      ),
    ),
    "card",
    s,
  );
}

function lookupBody(props) {
  const { lookup, localRoot, remoteRoot } = props;
  const status = lookup.status;
  // The AGREED time, not the file's mtime. `Identical here and on Proton Drive since 14:32 today.`
  // names the pass that last verified the two sides — the frame pins it at the daemon's last sync,
  // one minute after the `edited 14:31` on the card below it. Using the mtime would put the file's
  // own edit where the check belongs and say the two sides matched before they did.
  const verdict = verdictOf(status, props.agreedAt, lookup.error ?? null);

  const hero = el("div", { class: "activity-hero" });
  if (verdict.mark) {
    const mark = fid(renderHexagon({ size: SEAM_MARK, state: verdict.mark, masked: true }), "hexagon");
    for (const [i, path] of [...mark.querySelectorAll("path")].entries()) fid(path, "hexPath", i);
    hero.append(mark);
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
  // The daemon's exact string, in mono, never paraphrased — the same treatment S4 gives a failed
  // rehearsal and S5's own failed pass row.
  if (verdict.error) {
    const quoted = el("div", { class: "activity-hero-error" }, verdict.error);
    seamMask(quoted, { pad: 14, padY: 2, position: false });
    hero.append(quoted);
  }

  const seamBlock = fid(
    el(
      "div",
      { class: "activity-seam-block activity-seam-lookup" },
      fid(renderSeam({ site: "fileLookup" }), "seam"),
      fid(hero, "hero"),
    ),
    "seamBlock",
  );
  // 26px here against the quiet tab's 24px, and 34px for the block below — measured, and the only
  // two numbers on this screen that differ between its own two states for no reason but the drawing.
  seamBlock.style.marginTop = "26px";

  // The two cards are drawn only for a path the index actually knows. A miss has no sides to
  // describe, and a folder has no single size.
  const carded = status?.tracked && status.entity_kind !== "directory";
  const receivedAt = receivedAtFrom(status, props.receivedAt);
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
                // Absent far more often than the local clause — a downloaded file has no `up` row
                // at all — so the card is built to stand without it.
                meta: receivedAt ? `received ${receivedAt}` : null,
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
  //
  // THE TWO OPENERS ARE DRAWN ON WHAT PROVES THEM, one condition each, and not on the frame's
  // happy path. `Open folder` needs the index to say the file is here; `Open on Proton Drive` needs
  // the same `proton_id` the remote card needs, because without one there is no copy over there to
  // go and look at. A missing one is omitted by not appending it — `append(null)` writes the word.
  const footRow = [
    status?.tracked
      ? fid(
          smallSecondary(ACTIVITY.lookup.openFolder, () => props.onOpenFolder?.(lookup.path), OPENER_PAD),
          "openFolder",
        )
      : null,
    status?.proton_id
      ? fid(smallSecondary(ACTIVITY.lookup.openRemote, props.onOpenRemote, OPENER_PAD), "openRemote")
      : null,
    el("span", { class: "activity-spacer" }),
    status?.proton_id
      ? fid(
          el("span", { class: "activity-linked" }, ACTIVITY.lookup.linked(elideId(status.proton_id))),
          "linked",
        )
      : null,
  ].filter(Boolean);
  const tail = el("div", { class: "activity-content" });
  // The spacer on its own is not a row.
  if (footRow.length > 1) {
    tail.append(fid(el("div", { class: "activity-history-foot" }, ...footRow), "historyFoot"));
  }
  const lookupOpenError = openErrorLine(props.openError);
  if (lookupOpenError) tail.append(lookupOpenError);

  tail.style.marginTop = "34px";
  // The way back to a list this file was chosen out of. Drawn only when there IS one: a search that
  // matched a single file has no list behind it, and a button that returns to nothing is a dead end.
  if (props.chosen && (props.matches?.matches?.length ?? 0) > 1) {
    tail.prepend(
      el(
        "div",
        { class: "activity-match-back" },
        smallSecondary(ACTIVITY.lookup.backToMatches, props.onBackToMatches),
      ),
    );
  }
  return [seamBlock, fid(tail, "content")];
}

/**
 * `4c8f…9a21` — first four and last four of the NODE half of a composed id.
 *
 * `proton_id` is `volumeId~nodeId`; the volume half is the same for every file on the drive, so
 * eliding the whole string would show four characters that never change.
 */
/**
 * The path a typed query stands for — trimmed, and without a leading slash.
 *
 * Shared with `lookupPath` in app.js so the two agree on what "the same query" means; they would
 * otherwise disagree the moment someone types a leading space, and the count would never appear.
 */
export function normaliseQuery(query) {
  return String(query ?? "")
    .trim()
    .replace(/^\/+/, "");
}

/**
 * What a `search_files` reply means for the screen: the list it holds, and the one file to describe
 * — or null when there is not one.
 *
 * PURE, and separate from the fetch, because the three outcomes are exactly where this screen can
 * go wrong: one match must resolve straight to a verdict, several must NOT (a verdict about the
 * wrong `notes.md` is worse than a question), and none must carry the error through — a failed
 * search rendered as a miss tells someone their file is not being synced when nothing of the sort
 * is known.
 *
 * `query` is the BACKEND's, which is the resolved one: it expands `~` and strips the sync root, so
 * the miss names the path the index was actually asked about.
 */
export function searchOutcome(reply, typed, error = null) {
  const matches = reply?.matches ?? [];
  // `||`, NOT `??`: the backend resolves the sync root itself to the EMPTY string, and a miss card
  // headed by nothing is a hero with a blank line where the file's name goes. Falling back to what
  // was typed names the thing the user actually asked about.
  const query = reply?.query || normaliseQuery(typed);
  // `typed` as well as `query`, because they are two different questions. The count belongs to what
  // is IN the field — so the screen compares against what was typed — while the miss and the cards
  // name the file, which is the resolved path. A pasted `~/ProtonDrive/docs/spec.md` is both.
  const summary = { query, typed: normaliseQuery(typed), matches, total: reply?.total ?? matches.length };
  if (matches.length === 1) {
    return { matches: summary, lookup: { path: matches[0].path, status: matches[0].status, error: null } };
  }
  if (matches.length === 0) {
    return { matches: summary, lookup: { path: query, status: null, error } };
  }
  return { matches: summary, lookup: null };
}

export function elideId(protonId) {
  const node = String(protonId).split("~").pop() ?? "";
  return node.length <= 9 ? node : `${node.slice(0, 4)}…${node.slice(-4)}`;
}

// ----------------------------------------------------------------------------- passes tab ----

/**
 * Phrases that mean the daemon could not REACH Proton, as opposed to failing once it had.
 *
 * MEASURED AGAINST WHAT THE ENGINE CAN ACTUALLY SAY, not invented. `last_error` is mixed
 * provenance: some of it the engine wrote (`proton-drive {operation} timed out after {duration}`,
 * `src/proton.rs`) and the rest is the CLI's stderr passed through verbatim. Nothing classifies it —
 * `StatusHistoryEntry` carries the string and no cause — so this is a pattern match on prose, the
 * same shape and the same compromise as `gui-core`'s `looks_like_auth_error` (#103/E6 until the
 * daemon classifies its own failures).
 *
 * TIGHT ON PURPOSE, because the two errors are not symmetric. A miss labels a genuine outage
 * `Didn't finish`, which is quieter than the truth and still true; a false hit puts
 * `Couldn't reach Proton Drive` over a full disk, which is the bug #258 is about. So the default is
 * the neutral label and every entry here has to be transport vocabulary that a local failure has no
 * reason to use. `no space left on device`, `os error 2`, `session expired` and `permission denied`
 * all miss, and all four are failures with Proton perfectly reachable — the last of them reached it
 * and was refused, which is the case an auth-shaped split would have got backwards.
 *
 * The five in `activity.test.js` are the binding version of that sentence; this list is four of them
 * because a comment is not a test, and the day a needle starts matching one of them it is the test
 * that says so.
 */
export const UNREACHABLE_NEEDLES = [
  "timed out",
  "timeout",
  "connection",
  "network",
  "unreachable",
  "no route to host",
  "name resolution",
  "could not resolve",
  "offline",
];

/**
 * The label for a failed pass — the drawn one when the error names Proton, the neutral one always.
 *
 * Exported for the tests rather than for a second caller: what needs proving is the CLASSIFICATION,
 * and a test that goes through the row has to assert on rendered DOM to see it.
 *
 * NOT `looks_like_auth_error`, which is what #258 suggested. It is the wrong split for this label:
 * an expired session means Proton was reached and refused you, so the auth-shaped subset is exactly
 * the one that must NOT say `Couldn't reach Proton Drive` — and the frame's own error, a connection
 * timeout, does not match it, so following the suggestion literally would have flipped the drawn row
 * to the neutral label and failed the gate on the frame it was trying to be faithful to.
 */
export function failureLabel(error) {
  const message = String(error ?? "").toLowerCase();
  return UNREACHABLE_NEEDLES.some((needle) => message.includes(needle))
    ? ACTIVITY.passes.unreachable
    : ACTIVITY.passes.failed;
}

/**
 * One pass, as a row. `passRow` already matches the frame down to the failed variant's tinted
 * break-out and its quoted daemon string, so this is a mapping and not a builder.
 *
 * `detail` is the counters the pass reported; a pass with no summary has nothing to say about what
 * it moved, and an empty detail column is the truthful answer rather than `nothing to do`, which is
 * a claim that the pass ran and found nothing.
 */
function passRowFor(entry, retriedAt = null) {
  const failed = entry.last_error != null;
  return passRow({
    outcome: failed ? "failed" : "clean",
    label: failed ? failureLabel(entry.last_error) : ACTIVITY.passes.clean,
    // A FAILED PASS SAYS WHEN IT WAS PUT RIGHT, which is the frame's own `retried at 14:17 and
    // worked`. The retry is not a field on the entry — it is the next pass that succeeded — so a
    // failure with nothing after it yet has no such clause, and gets none rather than a guess.
    detail: failed ? (retriedAt ? ACTIVITY.passes.retried(retriedAt) : null) : detailOf(entry),
    time: clock(entry.epoch_secs),
    error: failed ? entry.last_error : null,
  });
}

/**
 * When each failed pass was put right — the clock time of the next pass that succeeded.
 *
 * Computed over the wire's own OLDEST-FIRST order, before the list is reversed for display, because
 * "the next one" is a fact about the order the passes actually happened in.
 */
function retriesIn(entries) {
  const at = new Map();
  for (const [i, entry] of entries.entries()) {
    if (entry.last_error == null) continue;
    const next = entries.slice(i + 1).find((e) => e.last_error == null);
    if (next) at.set(i, clock(next.epoch_secs));
  }
  return at;
}

/**
 * `2 sent, 1 brought here · 1 conflict kept` — the counters a pass reported, in the deck's terms.
 *
 * EVERY COUNTER, not the three the frames happen to draw. `PlanSummary` carries fourteen, and the
 * first version of this handled uploads, downloads and conflicts — which silently dropped the
 * frame's own `4 brought here · 1 move followed` row, whose `local_moves` rendered nothing. No gate
 * could see it: the detail span is not individually mapped, assert.mjs does not compare text, and
 * `.pass-detail` is a fixed 230px so even its box was right. A pass that did work and reports
 * nothing is exactly the wrong failure for this screen.
 *
 * Two clauses, separated by `·`: what MOVED, then what was DECIDED. `1 move followed` is the
 * frame's own wording; the rest follow it in register and are recorded in DEVIATIONS §77 as chosen
 * copy the deck can overrule.
 */
function detailOf(entry) {
  const s = entry.successful_sync_summary ?? entry.plan_summary;
  if (!s) return null;
  const moved = [
    s.uploads && `${count(s.uploads)} sent`,
    s.downloads && `${count(s.downloads)} brought here`,
    s.local_moves && `${count(s.local_moves)} ${plural(s.local_moves, "move", "moves")} followed`,
    s.remote_moves && `${count(s.remote_moves)} ${plural(s.remote_moves, "rename", "renames")} sent`,
    folders(s) && `${count(folders(s))} ${plural(folders(s), "folder", "folders")} made`,
  ].filter(Boolean);
  const decided = [
    s.conflicts && `${count(s.conflicts)} ${plural(s.conflicts, "conflict", "conflicts")} kept`,
    deletes(s) && `${count(deletes(s))} ${plural(deletes(s), "deletion", "deletions")} applied`,
    s.skipped_unsupported && `${count(s.skipped_unsupported)} skipped`,
  ].filter(Boolean);
  if (moved.length === 0 && decided.length === 0) return ACTIVITY.passes.nothingToDo;
  return [moved.join(", "), decided.join(", ")].filter(Boolean).join(" · ");
}

/** Folders made, either side — the two counters describe one thing a user would recognise. */
const folders = (s) => (s.remote_directories_created ?? 0) + (s.local_directories_created ?? 0);
/**
 * Deletions applied, either side. `purges` is NOT counted: it clears an index record and touches
 * no file, so putting it here would tell someone a pass deleted something when nothing was deleted.
 * `auto_links` and `type_conflicts` are left out for the same reason — neither moves or removes a
 * file, and this line is about what happened to files.
 */
const deletes = (s) => (s.remote_deletes ?? 0) + (s.local_deletes ?? 0);

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
  // `pillTabs` builds the two buttons, so they are stamped from the outside — the builder is shared
  // with S6 and has no business knowing about fid slots.
  fid(tabs.children[0], "filesTab");
  fid(tabs.children[1], "passesTab");
  const tabsSpacer = el("span", { class: "activity-spacer" });
  tabs.append(
    fid(tabsSpacer, "tabsSpacer"),
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
  // else entirely. G16.

  // NEWEST FIRST, which is the opposite of the wire. `6a Activity passes` draws 14:32 at the top
  // and 12:30 at the bottom; `status_history` arrives oldest-first. Reversed on a copy — the array
  // is the store's and `reverse()` mutates in place. The retry times are worked out BEFORE the
  // reversal, where "the next pass" still means the next one.
  const retried = retriesIn(history);
  const rows = history
    .map((entry, i) => fid(passRowFor(entry, retried.get(i) ?? null), "passRow", history.length - 1 - i))
    .reverse();

  const foot = fid(
    el(
      "div",
      { class: "activity-passes-foot" },
      fid(el("span", { class: "activity-retention" }, ACTIVITY.passes.retention), "retention"),
      el("span", { class: "activity-spacer" }),
      // 7px/14px, not the rung's 6px/13px: the four openers are drawn a pixel roomier than the
      // navigation buttons beside them in every frame that has one. `OPENER_PAD` is measured.
      fid(smallSecondary(ACTIVITY.passes.openLog, props.onOpenLog, OPENER_PAD), "openLog"),
    ),
    "passesFoot",
  );

  return [
    titleBlock(props.passesSub, { paddingTop: "6px", subGap: "7px" }),
    tabs,
    fid(
      el(
        "div",
        { class: "activity-passes" },
        ...rows,
        history.length ? foot : null,
        history.length ? openErrorLine(props.openError) : null,
      ),
      "passes",
    ),
  ];
}

// ---------------------------------------------------------------------------------- render ----

export function renderActivity(props) {
  const nodes = props.tab === "passes" ? passesTab(props) : filesTab(props);
  return nodes.filter(Boolean);
}

// ------------------------------------------------------------------------------- dialogs ----

/**
 * `7a Never synced` — both groups, enumerated.
 *
 * TWO GROUPS, TWO SOURCES, ONE SENTENCE EACH. The rule-matched half is `skip_rule_usage`, a walk of
 * the disk against the exclude globs. The `Can't be synced` half is `ControlResponse.unsyncable`,
 * the daemon's standing list — the entries the local stat-walk drops because they are not regular
 * files, which it now reports instead of skipping silently (#232). They are still not in the index
 * and never will be: the list is fed by the walk itself, not by anything that reads a record.
 *
 * Either group may be empty and each brings its own heading, so a machine with only sockets draws
 * only the second — the reason the group filter is `cannotSyncFrom` and not an `else`.
 */
export function renderNeverSyncedBody(props) {
  const never = props.never;
  const rules = never?.rules ?? [];
  const cannot = never?.cannot ?? { count: 0, rows: [] };
  const body = [];

  // ONE heading for the group, however many rules are in it. `7a Never synced` draws a single rule
  // so the frame cannot distinguish "per group" from "per rule" — but `You told it to skip these`
  // names the GROUP, and repeating it above every rule would read as several groups that happen to
  // share a title.
  if (rules.length > 0) {
    body.push(fid(eyebrow({ tone: "up", text: ACTIVITY.neverSyncedDialog.ruleHeading }), "ruleHeading"));
  }
  for (const [i, rule] of rules.entries()) {
    const pattern = fid(el("span", { class: "activity-rule-pattern" }, rule.pattern), "rulePattern", i);
    body.push(
      fid(
        el(
          "div",
          { class: "activity-rule-sub" },
          `${ACTIVITY.neverSyncedDialog.ruleSub("").trimEnd()} `,
          pattern,
        ),
        "ruleSub",
        i,
      ),
    );
    // `samples` is capped at MAX_SAMPLES per rule by the walk, so a rule matching thousands of
    // files lists the first few and the count above says how many there are in total.
    for (const [j, sample] of (rule.samples ?? []).entries()) {
      const row = pathRow({ path: sample.path, note: bytes(sample.bytes), mono: true });
      // The first row of each rule clears its sentence by 11px; the rest sit on the row rhythm.
      if (j === 0) row.style.marginTop = "11px";
      body.push(fid(row, "ruleRow", i, j));
      fid(row.children[0], "ruleRowPath", i, j);
      fid(row.children[1], "ruleRowNote", i, j);
    }
  }
  // One button too, and for the same reason: it opens the rules tab, which is where every one of
  // them is edited. N buttons pointing at one destination is N ways to do one thing.
  if (rules.length > 0) {
    const change = smallSecondary(ACTIVITY.neverSyncedDialog.changeRule, props.onChangeRule, {
      padding: "7px 14px",
    });
    change.style.marginTop = "12px";
    body.push(fid(change, "changeRule"));
  }

  // The second group. No button under it, and that is the point of its sub-line: there is no rule
  // to change and nothing to configure — a socket is not a file, and no setting makes it one.
  if (cannot.count > 0) {
    // `neutral`, NOT the `up` the rule heading takes: the frame draws this one at `--text-label`
    // (`#626b78`) against the other's amber. The amber marks the group you can do something about,
    // and there is no rule to change here — which is what this group's own sub-line says.
    const heading = eyebrow({ tone: "neutral", text: ACTIVITY.neverSyncedDialog.cannotHeading });
    // 26px clears the `Change this rule` button above it. Measured, and not the same gap as the
    // rule heading's, which clears a sentence rather than a control.
    if (body.length > 0) heading.style.marginTop = "26px";
    body.push(fid(heading, "cannotHeading"));
    body.push(
      fid(el("div", { class: "activity-rule-sub" }, ACTIVITY.neverSyncedDialog.cannotSub), "cannotSub"),
    );
    for (const [i, row] of cannot.rows.entries()) {
      // `dim` and NOT `mono`, which is the whole visual difference between the two groups and was
      // already written into `rows.css` waiting for this: a file you chose to skip reads at full
      // strength, one that cannot be synced at all reads quieter, and the note here is PROSE
      // (`a socket`) where the other group's is data (`2.1 MB`), so it stays sans. There is no size
      // to print either — a socket's `0 B` would read as an empty file someone could go and fix.
      const node = pathRow({ path: row.path, note: row.note, dim: true });
      if (i === 0) node.style.marginTop = "11px";
      body.push(fid(node, "cannotRow", i));
      fid(node.children[0], "cannotRowPath", i);
      fid(node.children[1], "cannotRowNote", i);
    }
  }

  return [
    fid(dialogBody({ padding: "0 24px", marginTop: "20px", children: body }), "dlgBody"),
    fid(
      dialogFoot({
        padding: "14px 24px 18px",
        marginTop: "14px",
        gap: "12px",
        align: "center",
        children: [
          fid(
            el("span", { class: "activity-reassurance" }, ACTIVITY.neverSyncedDialog.reassurance),
            "reassurance",
          ),
          el("span", { class: "activity-spacer" }),
          fid(filledSecondary(ACTIVITY.neverSyncedDialog.done, props.onClose), "done"),
        ],
      }),
      "dlgFoot",
    ),
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

  const kvRow = ([key, value], i) =>
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
    );

  return [
    fid(
      dialogBody({
        padding: "16px 22px 0",
        overflow: "hidden",
        // INSIDE THE BODY, not after the foot. `details` is the one dialog with a fixed height, and
        // `.dialog` clips what overflows it — a row appended below the foot would be invisible,
        // which is the failure this line exists to prevent. The body is `flex:1`, so it is 338 tall
        // either way and the message fits in the slack the eight rows leave.
        children: [...rows.map(kvRow), openErrorLine(props.openError)].filter(Boolean),
      }),
      "dlgBody",
    ),
    fid(
      dialogFoot({
        padding: "14px 22px 16px",
        marginTop: "12px",
        gap: "8px",
        children: [
          // `Copy all` needs no command — the clipboard is the webview's own. `Open the system log`
          // needs one, and has had it since #231: it snapshots `journalctl --user -u proton-syncd`
          // and opens that, because a journal is a binary store with no path to hand a file manager.
          //
          // 8px/14px on BOTH, not the 7px the same label wears on the passes tab: this dialog draws
          // its footer buttons 33 tall and that screen draws them 31.
          fid(
            smallSecondary(ACTIVITY.copyAll, () => copyDetails(rows), { padding: "8px 14px" }),
            "copyAll",
          ),
          fid(
            smallSecondary(ACTIVITY.passes.openLog, props.onOpenLog, { padding: "8px 14px" }),
            "detailsOpenLog",
          ),
        ],
      }),
      "dlgFoot",
    ),
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
/** The 48px one-way mark, with the gradient subtree the frame records under `defs`. */
function pendingMark() {
  // `masked`: the frame fills the track with `--surface`, the same way the settled marks do.
  const svg = renderHexagon({ size: PENDING_MARK, state: "syncing", direction: "up", masked: true });
  fid(svg, "pendingHexagon");
  fid(svg.querySelector("defs"), "pendingHexDefs");
  fid(svg.querySelector("defs > linearGradient"), "pendingHexGradient");
  const stops = svg.querySelectorAll("defs > linearGradient > stop");
  stops.forEach((stop, j) => fid(stop, "pendingHexStop", j));
  svg.querySelectorAll(":scope > path").forEach((path, i) => fid(path, "pendingHexPath", i));
  return svg;
}

export function renderFilePendingBody(props) {
  const t = props.transfer;
  const hero = fid(
    el(
      "div",
      { class: "activity-pending-hero" },
      pendingMark(),
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
    ),
    "pendingHero",
  );

  return [
    hero,
    fid(
      el(
        "div",
        { class: "activity-pending-foot" },
        fid(el("span", { class: "activity-pending-note" }, ACTIVITY.lookup.onlyLocal), "pendingNote"),
        el("span", { class: "activity-spacer" }),
        // The folder of the file being sent — the backend takes the file's own relative path and
        // computes the parent itself, so this never sends a directory it derived in JS.
        fid(
          smallSecondary(ACTIVITY.lookup.openFolder, () => props.onOpenFolder?.(t.path), OPENER_PAD),
          "pendingOpenFolder",
        ),
      ),
      "pendingFoot",
    ),
    // This dialog is CONTENT-SIZED (`routes.js`), so the reason grows it rather than being clipped.
    openErrorLine(props.openError),
  ].filter(Boolean);
}
