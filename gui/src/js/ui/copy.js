// The copy deck as a module (F7). Every user-visible string from `docs/design-v2/13-copy-deck.md`,
// verbatim, arranged in that file's own sections so the two can be read side by side.
//
// This matters more than a strings file usually does. THE COPY DECK IS THE SPEC — its opening line
// is "Every user-visible string, verbatim. Recreate exactly — the copy is load-bearing, especially
// around deletion" — the fidelity harness's copy gate asserts against these constants, and one
// module is what stops a sentence drifting between the screen, the tray and the notification that
// all quote it. Three surfaces quote "Nothing is lost. 4 changes are waiting…" today.
//
// PUNCTUATION IS PART OF THE STRING. The deck uses a typographic apostrophe (’) in "It's already
// gone", "Proton's Trash", "aren't copied", an en dash in ranges and an em dash in "Quiet is normal
// — most days". A straight quote is a copy-gate failure, not a typo, and it is exactly what the gate
// exists to catch.
//
// Voice rules, from the deck, for anything added later:
//   1. Say what happens to files, not what the daemon does.
//   2. Consequences in things you'd miss — "1,204 photos, 8.4 GB", not "directory, recursive".
//   3. Reassurance before the problem.
//   4. Never paraphrase a daemon error — exact string, in mono.
//   5. No exclamation marks, no emoji, no "Oops". Except the typed word DELETE.
//   6. "this computer", never a brand or OS name. "Proton Drive" in full, never "the cloud".
//   7. "kept" not "preserved", "waiting" not "pending", "brought here" not "downloaded" in prose.

import { count, cardinal, plural, bytes, clock } from "./format.js";

// ------------------------------------------------------------------ product and chrome ----

export const CHROME = {
  productName: "Proton Drive Sync",
  chips: {
    idle: "idle",
    syncing: "syncing",
    rehearsal: "rehearsal · nothing has changed",
    step: (n, of = 2) => `step ${n} of ${of}`,
    waiting: (n) => `${count(n)} waiting`,
  },
  doors: {
    activity: "Activity",
    plan: "Plan a sync",
    settings: "Settings",
    details: "Details",
  },
};

// -------------------------------------------------------------------------- main screen ----

export const MAIN = {
  settled: "Everything is up to date",
  settledSub: (ago, files, size) => `last synced ${ago} · ${count(files)} files · ${bytes(size)}`,
  /**
   * The Phase-1 form of the line above: the timestamp alone.
   *
   * No command reports an index-wide file count or byte total (G7, #207), and
   * `14-behaviour-and-state.md`'s rule for a missing capability is to omit the clause rather than
   * fill it — an em-dash where `12,480 files` goes would claim the daemon said "unknown" about
   * something nobody asked it. Deleted the day #207 lands, at which point `settledSub` is the line.
   */
  settledSubTime: (ago) => `last synced ${ago}`,
  /** `03-main-screen.md`: rows "cap at ~6 visible with `+n more` in mono if exceeded". */
  andMore: (n) => `+${count(n)} more`,
  syncing: (n) => `Syncing ${count(n)} ${plural(n, "change", "changes")}`,
  /**
   * `leaving == null` DROPS THE DIRECTION CLAUSE, for the same reason `unreachableBody` drops its
   * count: `last_plan_summary` is null until the plan exists, and on a first run against a large
   * tree that is the whole multi-minute scan-and-walk stretch — during which `syncing` is already
   * true. `0 leaving, 0 arriving` is a summary the daemon never published, and
   * `14-behaviour-and-state.md` is explicit that a null summary means unknown, never zero.
   */
  syncingSub: (ago, leaving, arriving) =>
    leaving == null
      ? `started ${ago}`
      : `started ${ago} · ${count(leaving)} leaving, ${count(arriving)} arriving`,
  otherWaiting: (n) => `${count(n)} other ${plural(n, "change is", "changes are")} waiting on you`,
  paused: "Paused",
  pausedSub: (n, since) =>
    `${count(n)} ${plural(n, "change has", "changes have")} piled up since ${since}. ` +
    "Nothing will move until you resume.",

  /**
   * The sign-in-expired hero, which no `2a` frame draws — split out of the ONE sentence the deck has
   * for this situation, `11-notifications.md`'s outage banner body:
   *
   *   `Proton Drive is asking you to sign in again. 61 changes are waiting — nothing is lost.`
   *
   * Two sentences, and the design's own division of labour puts the first in a headline and the
   * second in a sub-line — so the split is the deck's, not an invention, and both halves are still
   * checked verbatim against `11a Outage` (the copy gate matches a substring of a frame's text, and
   * `authExpiredSub` is in its drawn-template table at the count the frame draws).
   *
   * `routes.js` is why this exists at all: the onboarding latch releases on `authExpired`
   * specifically so the main screen can carry it, and a state that fell through to `Everything is up
   * to date` would be a false all-clear on a daemon that cannot reach Proton at all.
   */
  authExpired: "Proton Drive is asking you to sign in again",
  authExpiredSub: (n) => `${count(n)} ${plural(n, "change is", "changes are")} waiting — nothing is lost.`,

  /**
   * The hero for a pass that FAILED for any non-auth reason — #246, and no frame draws it either.
   *
   * It is the sibling of `PLAN.failedTitle`/`failedSub`: `14-behaviour-and-state.md`'s error table
   * specifies a failed rehearsal in prose — "show the daemon string" — and its testing checklist
   * ends on "Daemon error strings shown verbatim, never paraphrased". This is that treatment for
   * the pass rather than the rehearsal, so the two sentences here are S1's own and the third line
   * is the daemon's, quoted (voice rule 4).
   *
   * `didn't finish` RATHER THAN `failed`, and rather than the deck's `Can't reach Proton Drive`.
   * The first is voice rule 1 — what happened to the files, not what the daemon did. The second is
   * a claim we cannot make: `unreachable` means the app cannot reach the DAEMON, and a pass can
   * fail with Proton perfectly reachable (a full disk, a `proton-drive` binary that moved). The
   * quoted string underneath says which, and it says it in the daemon's words.
   *
   * `n == null` drops the count clause on `TRAY.unreachableBody`'s rule, and so does `0` — here
   * that is not "unknown", it is a genuinely empty watch queue, and `0 changes are waiting` sounds
   * like an all-clear in the one place that must not give one. The reassurance survives both.
   *
   * "Nothing is lost" IS TRUE OF A FAILED PASS, which is worth stating because the engine's own
   * checkpoint commits mean a half-finished pass DID move some files: what it never does is record
   * a side effect that did not happen, and the failed action re-plans next pass. So the promise is
   * about loss, not about inaction — which is why this does not borrow S4's `Nothing has been
   * touched`, a sentence that is true of a rehearsal and false of a pass.
   */
  failed: "The last sync didn't finish",
  failedSub: (n) =>
    n
      ? `Nothing is lost. ${count(n)} ${plural(n, "change is", "changes are")} waiting and will go on the next try.`
      : "Nothing is lost.",

  sideLocal: "This computer",
  sideRemote: "Proton Drive",
  sideRemoteCompact: "Proton",

  syncNow: "Sync now",
  pause: "Pause",
  resume: "Resume syncing",
  queued: "queued",

  footerPair: (local, remote) => `${local} ⇄ ${remote}`,
  footerTotals: (sent, received) => `${bytes(sent)} sent · ${bytes(received)} received today`,

  /**
   * The attention band's two rows. `2a Needs you` draws them at one conflict and two deletions, and
   * S1 renders them from the live counts — so the three strings that carry a number are functions
   * here, where the deck writes the drawn instance.
   *
   * THAT COSTS COPY-GATE COVERAGE AND BUYS IT BACK, deliberately. `copy-gate.mjs` compares string
   * constants only — a template cannot be checked without inventing its arguments — so turning these
   * into templates would quietly drop three of the deck's sentences from the gate. F7's own note says
   * as much. The gate now carries a `DRAWN` table of templates WITH the arguments the frame draws,
   * and these three are in it: `conflictTitle(1)`, `deletionTitle(2)` and `deletionSub(1, 1)` are
   * still asserted verbatim against `2a Needs you`, and now so are six templates that were never
   * checked at all.
   *
   * `deletionSub` DROPS A ZERO CLAUSE rather than printing it. A queue of two permanent deletions
   * saying `2 remove from this computer permanently · 0 go to Proton's Trash` names a thing that is
   * not happening, in the sentence whose whole job is telling you what you are about to lose.
   */
  band: {
    conflictTitle: (n) =>
      n === 1 ? "One file changed on both sides" : `${cardinal(n)} files changed on both sides`,
    conflictSub: (path) => `${path} · both copies kept, nothing lost`,
    conflictAction: "Compare",
    deletionTitle: (n) =>
      n === 1 ? "One deletion is waiting on you" : `${cardinal(n)} deletions are waiting on you`,
    deletionSub: (permanent, trash) =>
      [
        permanent > 0
          ? `${count(permanent)} ${permanent === 1 ? "removes" : "remove"} from this computer permanently`
          : null,
        trash > 0 ? `${count(trash)} ${trash === 1 ? "goes" : "go"} to Proton's Trash` : null,
      ]
        .filter(Boolean)
        .join(" · "),
    deletionAction: "Review",
  },

  compact: {
    upToDate: "Up to date",
    needYou: (n) => `${count(n)} ${plural(n, "thing needs", "things need")} you`,
    conflictLine: "One file changed on both sides.",
    deletionLine: "Two deletions are waiting.",
    review: "Review them",
    syncingContinues: "syncing continues",
    later: "Later",
    open: "Open",
  },
};

// ---------------------------------------------------------------------------- conflicts ----

export const CONFLICTS = {
  title: "You both changed this file",
  sub: "Nothing has been lost — both versions are still here, and syncing carries on around this.",
  position: (i, of) => `${i} of ${of}`,

  /**
   * The line under the filename. Drawn `a plain text file · last agreed 3 hours ago`; Phase 1 can
   * only write the first half.
   *
   * `last agreed` is the baseline's timestamp, and there isn't one: `FileRecord` has no
   * last-synced field, and the daemon's conflict arm overwrites the original's record with the
   * CURRENT local state, so even the mtime proxy is gone by the time the GUI could read it. Same
   * missing capability as the cards' first line — #217. The clause is dropped rather than
   * em-dashed, per `14-behaviour-and-state.md`'s rule for a missing value inside a sentence.
   */
  meta: (kind) => kind,
  /**
   * The three answers to "what sort of file is this", and deliberately only three.
   *
   * Nothing distinguishes an SVG from a note — an SVG is valid UTF-8 and reads exactly the same
   * way — so naming a type we cannot tell apart would be worse than naming a category.
   */
  kindText: "a plain text file",
  kindBinary: "a file this app can't read",
  // NOT the queue row's wording, which is `typeConflict` below and reads `a folder here, a file
  // there`. This is the META LINE — the slot that says `a plain text file` on `3a Conflict`, where
  // the sentence stands alone under a filename rather than beside a path in a list, and can afford
  // to name the far side. Two strings for one situation, drawn in two places, and only one of the
  // two places is drawn: no frame opens a type conflict, so this one stays in the gate's NOT_DRAWN
  // table while `typeConflict` is measured off `3a Conflict diff`.
  kindFolder: "a folder here, a file on Proton Drive",

  /** The metadata row's other two items. Both are counted off the pair, so both take their number. */
  lineCount: (n) => `${count(n)} ${plural(n, "line", "lines")}`,
  edited: (epochSecs) => `edited ${clock(epochSecs)}`,

  mine: "Your version · this computer",
  theirs: "Proton's version · from another device",
  // The diff view's column labels — the same two words without the device clause, because the
  // panel below them already says which side is which by colour and position.
  mineShort: "Your version",
  theirsShort: "Proton's version",

  /**
   * The card's second line — "what differs, in words" — from the facts `ui/diff.js` extracts.
   *
   * Two sentences are drawn and they are not the same shape: the one with a lone changed line
   * contrasts the sides and closes with "otherwise the same", the one that also gained a line
   * names the gain instead. Both are this grammar at different facts, which is the only reason it
   * is a grammar rather than two strings — a sentence nothing draws is a sentence nobody checked.
   *
   * `quoted` renders inline mono inside the sentence; the caller splits around it with
   * `splitEmphasis`, the same way the attention band's bold spans work.
   *
   * **`null` in, `null` out.** `summariseSide` returns null whenever the comparison is not one this
   * grammar covers — which includes the most common real conflict, a multi-line edit — so null is
   * the ordinary case rather than an error, and the caller's `if (sentence)` is exactly the branch
   * `04-conflicts.md` specifies (the metadata row, alone). Throwing here would put a TypeError on
   * the path S2 travels most, and a TypeError in a webview is a blank card.
   */
  versionDiff: (side, facts) => {
    if (!facts) return null;
    const ours = side === "mine" ? "Yours" : "Proton's";
    const theirs = side === "mine" ? "Proton's" : "yours";
    const extra =
      facts.extraAtEnd === 1
        ? "an extra line at the end"
        : `${count(facts.extraAtEnd)} extra lines at the end`;
    if (facts.quoted && facts.otherwiseSame) {
      return `${ours} has ${facts.quoted} where ${theirs} has something else, and is otherwise the same.`;
    }
    if (facts.quoted) return `${ours} has ${facts.quoted} and ${extra}.`;
    return `${ours} has ${extra}.`;
  },

  /**
   * The first line — "what happened" — which no version of this app can currently generate.
   *
   * `You added a line` is a claim about your version against the LAST AGREED one, and the last
   * agreed version's content exists nowhere: the sidecar is Proton's copy as it is now, and the
   * index keeps the baseline's SHA-1 without its bytes. Against Proton's copy alone the very same
   * edit reads as a removal. Kept as the drawn constants, unused until something records a common
   * ancestor; `ui/diff.js` says the same thing at more length.
   */
  mineChange: "You added a line, 5 minutes ago",
  theirsChange: "Changed a line and added one, 2 minutes ago",

  showDiff: "See the exact differences",
  hideDiff: "Hide differences",
  openBoth: "Open both in an editor",

  keepMine: "Keep mine",
  keepMineSub: "Your version goes to Proton Drive. Proton's version is discarded.",
  keepBoth: "Keep both",
  /**
   * The sidecar's real name, quoted back. `.proton-cloud` is a suffix users see on disk and search
   * for, so the sentence naming the actual file is the point of it — a generic
   * "as a .proton-cloud file" would be the one version of this sentence that does not help.
   */
  keepBothSub: (sidecar) => `Nothing is lost. Proton's copy lands beside yours as ${sidecar}.`,
  useTheirs: "Use Proton's",
  useTheirsSub: "Proton's version replaces the file on this computer. Yours is discarded.",

  cannotUndo: "Discarding a version can't be undone from here.",
  later: "Decide later",

  /**
   * The disclosure's header and footer, both counted off the same comparison the cards use.
   *
   * **Zero is refused, not rendered.** `cardinal(0)` is the deliberately lower-cased `zero`, so the
   * header would open a sentence with `zero lines differ.` — and it is reachable: a sidecar written
   * with different line endings, or one trailing newline, differs as bytes while no line differs
   * (`diff.js`'s `invisibleDifference`). A conflict exists there, so `0 lines differ · 2 lines
   * identical` under a heading that says the file matches is the reassuring-direction lie the cards
   * already refuse. Null lets the caller keep the disclosure shut.
   */
  diffSummary: (differing) =>
    differing > 0
      ? `${cardinal(differing)} ${plural(differing, "line differs", "lines differ")}. Everything else in the file matches.`
      : null,
  diffCounts: (differing, identical) =>
    differing > 0
      ? `${count(differing)} ${plural(differing, "line differs", "lines differ")} · ` +
        `${count(identical)} ${plural(identical, "line identical", "lines identical")}`
      : null,
  /**
   * The placeholder on the side that does not have the line.
   *
   * TAKES A SIDE, because the drawn string is only half the pair. `3a Conflict diff` puts an extra
   * line on Proton's side, so the placeholder lands on the LEFT and reads `not in your version` —
   * but `alignedRows` emits the mirror shape for any line yours has and Proton's does not, and
   * there the same sentence is false. The deck has no drawn twin (`copy-gate.mjs` carries the
   * reason), so this is the one sentence here written rather than measured.
   */
  absentLine: (side) => (side === "mine" ? "not in your version" : "not in Proton's version"),

  stillWaiting: "Still waiting after this one",
  bothChanged: "both changed it",
  typeConflict: "a folder here, a file there",

  clearedTitle: "Nothing left to decide",
  /**
   * What you just did, counted off the choices this screen actually made.
   *
   * A CONSTANT HERE WOULD BE A LIE THE MOMENT ANYONE SETTLED A DIFFERENT NUMBER. The frame draws
   * `You settled 3 files. Two kept both versions, one took Proton's copy.` — three specific counts
   * of three specific resolutions — and an empty scan carries no memory of any of it, so the
   * screen tracks its own session and renders from that.
   *
   * The breakdown clause is DROPPED rather than approximated when the deck has no wording for the
   * mix: `Keep mine` and `Decide later` appear in no drawn sentence, and inventing grammar for
   * them here would put copy in the module whose whole job is not to. The first sentence is always
   * true, which is the half worth keeping.
   */
  clearedSub: ({ total = 0, keptBoth = 0, tookProton = 0 } = {}) => {
    const opening = `You settled ${count(total)} ${plural(total, "file", "files")}.`;
    const parts = [];
    // `versions` ALWAYS, and never `plural(keptBoth, …)`. The noun agrees with `both`, which is
    // inherently two — the two versions of one file — while `keptBoth` counts FILES. Agreeing it
    // with the file count gave `one kept both version` for a single file, which is the reading
    // where `both` has silently become a count of files rather than of versions.
    if (keptBoth > 0) parts.push(`${cardinal(keptBoth).toLowerCase()} kept both versions`);
    if (tookProton > 0) parts.push(`${cardinal(tookProton).toLowerCase()} took Proton's copy`);
    if (!parts.length || keptBoth + tookProton !== total) return opening;
    // The frame capitalises the first breakdown clause and lower-cases the second.
    const [first, ...rest] = parts;
    const sentence = [first.charAt(0).toUpperCase() + first.slice(1), ...rest].join(", ");
    return `${opening} ${sentence}.`;
  },
  back: "Back to sync",
};

// ---------------------------------------------------------------------------- deletions ----

/** Shared by the two armed-confirmation sentences below; see `armedBody`. */
const ARMED_TAIL =
  "It does not go to your trash, and it is already gone from Proton Drive, " +
  "so there is nothing to restore it from.";

export const DELETIONS = {
  /**
   * `Two files are waiting to be deleted` — a TEMPLATE, because the queue is live and one deletion
   * is the commonest case there is.
   *
   * The count is spelled (`cardinal`), which is the deck's own form here and nowhere else on this
   * screen: the sentence is prose and `2 files are waiting` reads as a readout. Agreement moves with
   * it — `One file IS waiting` — the same failure `plural` was written for after `1 other changes
   * are waiting on you` shipped once.
   */
  title: (n) => `${cardinal(n)} ${plural(n, "file is", "files are")} waiting to be deleted`,
  sub: "They were deleted on one side. Nothing happens to the other side until you say so — syncing carries on around them.",

  permanent: "Permanent · this computer",
  permanentSub: "Removed straight from disk. Not moved to any trash, and not recoverable from Proton.",
  recoverable: "Recoverable · Proton Drive",
  recoverableSub: "Moved to Proton Drive's Trash. You can restore it there until the trash is emptied.",

  /**
   * The permanent card's consequence, in two forms — and they are two SENTENCES, not one sentence
   * with a hole in it.
   *
   * `folderConsequence` is what the frame draws: the subtree aggregate, emphasised, because
   * `1,204 photos, 8.4 GB` is voice rule 2 exactly ("consequences in things you'd miss"). No command
   * can produce it — G8, #208 — so Phase 1 cannot render this string at all.
   *
   * `folderConsequenceUnknown` is what Phase 1 says instead. It is a different sentence rather than
   * the same one with the number omitted, because "Deleting this removes from this computer" is not
   * English and "Deleting this removes this folder" says nothing. It keeps the emphasis on the LOSS
   * — `everything inside it`, which is true, qualitative, and the part a reader is weighing — so the
   * card keeps its crimson span and its structure. The day #208 lands this one goes.
   */
  folderConsequence: (loss) =>
    `Deleting this removes ${loss} from this computer, including everything inside it.`,
  folderConsequenceUnknown: "Deleting this folder removes everything inside it from this computer.",
  /** The same sentence for a single FILE, which the frame does not draw and the queue can hold. */
  fileConsequence: "Deleting this removes it from this computer for good.",
  travelExplainer:
    "You deleted this on this computer. Deleting it on Proton moves it to Proton Drive's Trash, where you can still get it back.",

  /**
   * The facts strip, in mono, under the divider. Templates because two of the four are relative
   * times off `detected_epoch_secs` and one is a month off the index's `mtime`; the deck writes them
   * out (`13-copy-deck.md`, Deletions) and F7 left them out, which nothing caught because the copy
   * gate reads copy.js and not the deck.
   *
   * `lastOpened` is an ATIME and the index stores mtime only (#208) — it is here because the deck
   * has it and the screen must be able to say where it went, not because Phase 1 can draw it.
   */
  deletedOnProton: (ago) => `deleted on Proton ${ago}`,
  deletedHere: (ago) => `deleted here ${ago}`,
  lastOpened: (when) => `last opened ${when}`,
  lastEdited: (when) => `last edited ${when}`,

  typeToDelete: "To delete it, type DELETE below.",
  delete: "Delete",
  toTrash: "Move to Proton's Trash",
  keepRemote: "Keep it — put it back on Proton Drive",
  keepLocal: "Keep it — bring it back to this computer",
  noExpiry: "Deletions stay here until you decide. Nothing expires.",
  keepBoth: "Keep both files",

  /**
   * `Delete 1,204 photos from this computer?` — ONE grammar, and the caller names the thing.
   *
   * The frame's noun phrase is the subtree aggregate and Phase 1 passes the path instead
   * (`Delete photos/2019 from this computer?`), which is the same sentence about the same deletion
   * and invents nothing. Written this way rather than as two templates because a confirmation
   * question is the last place two copies of one sentence should be able to drift.
   */
  armedTitle: (what) => `Delete ${what} from this computer?`,
  /**
   * The half of the armed sentence that is the same whichever kind of thing is going. Module-local,
   * so the copy gate walks the two whole sentences below rather than a fragment neither screen ever
   * draws on its own.
   */
  /**
   * `size == null` DROPS THE SIZE CLAUSE, the shape `MAIN.syncingSub` uses for a null plan summary.
   * The frame draws `— 8.4 GB —` from the same subtree aggregate the title needs (#208); an
   * em-dash pair around an em-dash would be the app claiming the daemon answered "unknown" about
   * how much is at stake, on the one screen where that is the question.
   */
  armedBody: (path, size) =>
    `Everything in ${path}${size == null ? "" : ` — ${size} —`} is removed from disk. ${ARMED_TAIL}`,
  /**
   * The same confirmation for a single FILE, which no frame draws — both drawn permanent items are
   * folders. `Everything in archive/old-notes.md` is folder grammar about a file, and the takeover
   * is the last thing a person reads before something goes for good, so it gets its own sentence
   * rather than a shared one that is wrong half the time.
   */
  armedBodyFile: (path) => `${path} is removed from disk. ${ARMED_TAIL}`,
  armedConfirm: "Delete permanently",
  armedCancel: "Press Esc to cancel.",

  emptyTitle: "Nothing waiting to be deleted",
  emptySub:
    "When a file disappears from one side, it waits here for you instead of vanishing from the other.",

  compact: {
    title: (n) => `${count(n)} ${plural(n, "file", "files")} waiting to be deleted`,
    permanent: "1,204 photos gone from this computer, permanently",
    recoverable: "to Proton's Trash — recoverable",
    review: "Review them",
  },
};

// -------------------------------------------------------------------------- plan a sync ----

// Eight of these turned from constants into templates. A constant that becomes a template leaves the
// copy gate silently, so each one is listed in the gate's `DRAWN` table at the arguments its frame
// draws it at.
export const PLAN = {
  title: (n) => `The next sync moves ${count(n)} ${plural(n, "thing", "things")}`,
  // `n` is the DESTRUCTIVE count, not the total. At 0 the clause is dropped rather than rendered as
  // `Zero of them`; the rehearsal sentence stands on its own.
  sub: (n) =>
    n > 0
      ? `${cardinal(n)} of them can't be undone. Everything here is a rehearsal — nothing has changed yet.`
      : "Everything here is a rehearsal — nothing has changed yet.",
  checkAgain: "Check again",

  leaving: "Leaving this computer",
  arriving: "Arriving from Proton",
  // The unit only: the frame draws the count as its own 42px span, so `3` and `files, 4.1 MB` are
  // two nodes — a template holding both would put the numeral in the 14px tier. `size` is a
  // preformatted string (`4.1 MB`) or null; Phase 1 always passes null, since no dry-run field
  // carries a byte total (G2, #191), and the clause is omitted rather than faked.
  sideUnit: (n, size = null) => `${plural(n, "file", "files")}${size ? `, ${size}` : ""}`,
  plusFolder: (n) =>
    `Plus ${cardinal(n).toLowerCase()} new ${plural(n, "folder", "folders")} created on Proton Drive to hold them.`,
  plusRename: (n) =>
    `${cardinal(n)} ${plural(n, "file", "files")} you renamed will be renamed here to match.`,
  // The mirrors of the two above: a folder can be created on either side and a rename applied on
  // either side. No frame draws these two, so they are not in the copy gate (both are templates,
  // which `NOT_DRAWN` cannot hold); gui/test/plan.test.js pins them.
  plusFolderHere: (n) =>
    `Plus ${cardinal(n).toLowerCase()} new ${plural(n, "folder", "folders")} created on this computer to hold them.`,
  plusRenameThere: (n) =>
    `${cardinal(n)} ${plural(n, "file", "files")} you renamed will be renamed on Proton Drive to match.`,

  // `kind` exists because `plan_sync` emits `RemoteDelete`/`LocalDelete` with
  // `EntityKind::Directory` for a whole subtree, so the band's subject can be a folder; calling that
  // a file understates the loss. `thing` is the mixed case, where either noun is wrong half the time.
  destructiveTitle: (n, kind = "file") =>
    `${cardinal(n)} ${plural(n, kind, `${kind}s`)} ${plural(n, "gets", "get")} deleted for good`,
  // The deck draws one sentence, the `remote_delete`; `destructiveLocal` is the same sentence with
  // the sides swapped, the mirror `05-deletions.md` builds its two columns on. `inside` is appended
  // rather than folded into the subject so the mono span still wraps the path alone.
  destructiveRemote: (path, inside = false) =>
    `${path}${inside ? " and everything inside it" : ""} is removed from Proton Drive. It's already gone from this computer, so nothing will bring it back.`,
  destructiveLocal: (path, inside = false) =>
    `${path}${inside ? " and everything inside it" : ""} is removed from this computer. It's already gone from Proton Drive, so nothing will bring it back.`,
  // More than one: no path, because naming them all would run past the band. The tinted rows below
  // carry the paths.
  destructiveMany: (n, kind = "file") =>
    `${count(n)} ${kind}s are removed for good. Nothing will bring them back.`,
  leaveItAlone: "Leave it alone",

  everyAction: "Every action, in order",
  // The conflict clause is conditional: at 0 it is dropped, not rendered as `0 conflicts`.
  actionSummary: (n, conflicts = 0) =>
    conflicts > 0
      ? `${count(n)} ${plural(n, "action", "actions")} · ${count(conflicts)} ${plural(conflicts, "conflict", "conflicts")} kept as both copies`
      : `${count(n)} ${plural(n, "action", "actions")}`,

  gate: "type DELETE to allow it",
  gateWhy: "Only needed because this plan deletes something.",
  runWithout: "Run it without the deletion",
  run: "Run this sync",

  safeTitle: "Nothing gets deleted",
  // `n` is the files that MOVE (uploads + downloads, the two numbers the seam block counts), not the
  // action count: `5a Plan safe` draws seven actions and says `Five files move`, because a new folder
  // and a rename are neither arriving nor leaving. The verb agrees as well as the noun, and 0 reads
  // `No files` — a plan can be entirely a new folder or a rename (the empty plan is `nothing*`).
  safeSub: (n) =>
    `${n === 0 ? "No files" : `${cardinal(n)} ${plural(n, "file", "files")}`} ${plural(n, "moves", "move")}, both sides end up with everything. This plan is safe to run.`,
  // The empty plan: `14-behaviour-and-state.md` routes it to the safe variant ("Plan · Empty:
  // safe-plan variant"), but no frame draws it, so these two sentences are S4's rather than the
  // deck's (copy-gate `NOT_DRAWN`). `safeTitle`/`safeSub` cannot be reused: `Nothing gets deleted`
  // over `No files move` says nothing happened three times.
  nothingTitle: "Nothing needs to move",
  nothingSub: "Both sides already match. Running this sync now would change nothing.",
  newFolder: "new folder",
  moved: "moved",
  checkedAgo: (ago) => `Checked ${ago} against both sides.`,

  checkingTitle: "Working out what would change",
  checkingSub: "Comparing both sides. Nothing is being touched.",
  checkingProgress: (done, total) => `${count(done)} of ${count(total)} files`,
  stop: "Stop",

  // The failed rehearsal. `14-behaviour-and-state.md`'s empty-and-error table specifies the state in
  // prose ("dry run failed → show the daemon string, offer `Check again`") and no frame draws it, so
  // these two are S4's rather than the deck's (copy-gate `NOT_DRAWN`). The title is `checkingTitle`
  // in the past tense. Neither paraphrases the error (voice rule 4) — they introduce it, and the
  // daemon's exact string is rendered beneath in mono with no formatter anywhere in the app.
  failedTitle: "Couldn't work out what would change",
  failedSub: "Nothing has been touched. This is what it said:",
};

// ----------------------------------------------------------------------------- activity ----

export const ACTIVITY = {
  title: "Activity",
  quietSub: (since, ago) =>
    `Nothing has needed to move since ${since}. Both sides matched at the last check, ${ago}.`,

  lookupPlaceholder: "Check a file — type any name or path",
  lookupShortcut: "Ctrl F",
  matches: (n) => `${count(n)} match${n === 1 ? "" : "es"}`,

  agree: "Both sides agree",
  watched: (ago) => `watched continuously · checked ${ago}`,
  nextCheck: (inTime) => `next full check in ${inTime}`,

  neverSyncedTitle: (n) => `${count(n)} ${plural(n, "file is", "files are")} never synced`,
  /**
   * Two clauses, and the second one goes when its group does.
   *
   * Drawn at (2, 2). The `cannot` group — sockets and symlinks — has no Phase-1 source at all
   * (nothing that is not a file ever enters the index), so live this renders with `cannot: 0` and
   * the sentence has to stop after the first clause rather than claim `zero can't be synced`.
   */
  neverSyncedSub: (byRule, cannot) => {
    const head = `They sit in your folder but aren't copied anywhere.`;
    const first = `${cardinal(byRule)} ${plural(byRule, "matches", "match")} a rule you wrote`;
    if (!cannot) return `${head} ${first}.`;
    return `${head} ${first}; ${cardinal(cannot, "mid")} can't be synced at all.`;
  },
  showThem: "Show them",

  lastToMove: "Last things to move",
  lastToMoveSub: (n, days) =>
    `${count(n)} ${plural(n, "file", "files")} in the last ${count(days)} ${plural(days, "day", "days")}`,
  quietIsNormal: "Quiet is normal — most days nothing needs to move.",
  allFiles: (n) => `All ${count(n)} ${plural(n, "file", "files")}`,
  passesTab: "Sync passes",
  filesTab: "Files",
  nothingRecent: "Nothing has moved in the last hour.",

  lookup: {
    safe: "Safely on both sides",
    safeSub: (at) => `Identical here and on Proton Drive since ${at} today.`,

    // THE FOUR VERDICTS NO FRAME DRAWS. `path_sync_status` answers `synced` / `modified` /
    // `conflict`, and also reports `tracked: false` and `entity_kind: "directory"` — five outcomes
    // reachable from the first thing anyone types, of which the frames draw exactly one. Undrawn
    // states are where every bug on this project has lived, so they are written here rather than
    // improvised at the call site, and pinned by a unit test the way S4 pinned `PLAN.destructive*`.
    changed: "Changed here, not sent yet",
    changedSub: "This computer has the newer copy. The next sync sends it.",
    conflict: "Both sides changed",
    conflictSub: "Both copies were edited. Nothing is lost — Conflicts decides which one to keep.",
    folder: "That's a folder",
    folderSub: "Folders are followed as a whole. Look up a file inside it to see where it stands.",
    // 14-behaviour-and-state.md:130, verbatim. The miss is the ordinary case, not an error: the
    // lookup matches a relative path exactly, so a bare name that is not at the root misses.
    noMatch: "No file by that name in your sync folder.",
    noMatchSub: "Type the path as it sits inside your sync folder, like docs/spec.md.",
    // A FAILED CHECK IS NOT A MISS, and conflating the two is the worst answer this screen can
    // give: "no file by that name" about a file that is sitting right there would tell someone
    // their file is not being synced when nothing of the sort is known. Same shape as S4's failed
    // rehearsal — say the check failed, then quote the daemon exactly, in mono (voice rule 4).
    failed: "Couldn't check that file",
    failedSub: "Nothing has been changed. This is what it said:",
    // A status this build has no verdict for. `failed` is the honest title — the check ran, and we
    // still cannot say where the file stands — and the raw value is quoted in mono beneath, because
    // it is the one thing that makes the report actionable.
    unknownSub: "The daemon reported a state this version does not know about:",
    history: "This file's history",
    sent: "Sent to Proton Drive",
    keptYours: "Both sides had changed — you kept yours",
    firstBrought: "First brought to this computer",
    openFolder: "Open folder",
    openRemote: "Open on Proton Drive",
    linked: (id) => `linked · id ${id}`,
    pending: "On its way to Proton Drive",
    pendingSub: (ago, size) => `Started ${ago} · ${bytes(size)}`,
    onlyLocal: "only on this computer so far",
  },

  neverSyncedDialog: {
    title: (n) => `${count(n)} ${plural(n, "file is", "files are")} never synced`,
    sub: "They live in your folder but no copy exists on Proton Drive.",
    ruleHeading: "You told it to skip these",
    ruleSub: (pattern) => `A rule in your settings matches them: ${pattern}`,
    changeRule: "Change this rule",
    cannotHeading: "Can't be synced",
    cannotSub: "Not real files — Proton Drive has nothing to store for them.",
    socket: "a socket",
    shortcut: "a shortcut",
    reassurance: "Nothing here is at risk — it's just not backed up.",
    done: "Done",
  },

  passes: {
    /**
     * Drawn at (18, 20, 1, true). Every number in it is live — `status_history` holds up to
     * `STATUS_HISTORY_LIMIT` (20) entries and clean-vs-failed is `last_error == null` — so it
     * cannot stay the constant it was: against the shipped six-entry fixture the true sentence is
     * `5 of the last 6 …`.
     *
     * `recovered` is the one argument that is not a count. "retried on its own" is a claim that a
     * LATER pass succeeded, which is a fact about the ORDER of the history and not about any one
     * entry; when the newest pass is itself the failure there has been no retry yet, and saying so
     * would be false in the one state where the user most needs the truth.
     */
    summary: (clean, total, failed, recovered) => {
      if (!failed) return `All ${count(total)} recent ${plural(total, "pass", "passes")} finished cleanly.`;
      const head = `${count(clean)} of the last ${count(total)} passes finished cleanly.`;
      if (!recovered) return `${head} The most recent one failed.`;
      return `${head} ${cardinal(failed)} failed and ${plural(failed, "retried on its own", "retried on their own")}.`;
    },
    chartTitle: "Last 20 passes",
    chartSub: (from) => `how long each took · ${from} onward`,
    mostRecent: (at) => `most recent ${at}`,
    clean: "Finished cleanly",
    unreachable: "Couldn't reach Proton Drive",
    retried: (at) => `retried at ${at} and worked`,
    // Voice rule 4: this is a DAEMON STRING and appears exactly as the daemon said it, in mono.
    // It is here only because the deck quotes it as the example; never construct one of these.
    exampleDaemonError: "proton-drive: connection timed out after 60s",
    nothingToDo: "nothing to do",
    retention: "Only the last 20 passes are kept. Anything older lives in the system log.",
    openLog: "Open the system log",
  },

  copyAll: "Copy all",
};

// ----------------------------------------------------------------------------- settings ----

export const SETTINGS = {
  title: "Settings",
  sub: "Changes here take effect on the next sync. Nothing is written until you save.",
  tabs: {
    folders: "Folders",
    skip: "What to skip",
    deletions: "Deletions",
    advanced: "Advanced",
  },

  pairTitle: "The pair being kept in step",
  choose: "Choose…",
  pairLocalNote: (files, size) =>
    `${count(files)} files, ${bytes(size)} in here today. Changing it starts a fresh merge — nothing gets deleted.`,
  // The drawn sentence minus its opening clause. `12,480 files, 41.2 GB` is G7 (#207) — no command
  // reports the folder's totals, and `skip_rule_usage`'s `considered_files` is a count with no byte
  // twin, so half of it would still be missing. What is left is the half that matters: a warning
  // about what changing the folder does. Same shape as `DELETIONS.folderConsequenceUnknown`.
  pairLocalNoteUnknown: "Changing it starts a fresh merge — nothing gets deleted.",
  pairRemoteNote: "Folder on your Proton Drive. Must already exist.",

  cadenceTitle: "How often it checks",
  eventsDriven: "Notice changes the moment they happen",
  eventsDrivenSub:
    "Proton tells the app when something changes on another device, so it syncs within seconds.",
  fullScan: "Compare everything, top to bottom",
  fullScanSub: (files) =>
    `A full check of all ${count(files)} files as a safety net. It's slow, so it runs on a schedule rather than constantly.`,
  weekly: "Weekly",
  monthly: "Monthly",
  every: "Every",
  at: "at",
  onDay: "On day",
  monthEdgeNote: "Months without a 15th are skipped to the last day.",
  // The second panel, as Phase 1 can honestly title it. G4 (#193): there is no `full_scan_schedule`
  // key, no scheduler and no command — and `events_full_scan_every`, the nearest real thing, counts
  // PASSES rather than days and defaults to off. So the panel keeps its shell and changes its
  // subject to the one cadence the config does carry, `scan_interval_secs`.
  //
  // NOT the drawn title with a different control under it: `Compare everything, top to bottom` over
  // a timer that (with live updates on) runs an incremental pass would be a false claim about what
  // the app does with someone's files, which is the one thing this screen may not do.
  timer: "Look for changes on a timer",
  timerSub: "A backstop for the live updates above. It runs whether or not Proton reported anything.",
  timerUnit: (mins) => `${mins} min`,
  timerSeconds: (secs) => `${secs} s`,

  runOne: "Run one now",
  fullSweep: "Full sweep now",
  fullSweepNote:
    "Takes about 4 minutes; syncing keeps working. Last one 2 days ago — nothing was out of step.",
  // Both dropped clauses are G18 (#238): no per-pass duration exists anywhere, and `status_history`
  // does not record which past pass was a full sweep — so `Takes about 4 minutes` and `Last one 2
  // days ago` have no source and no near neighbour. What survives is what is true every time.
  fullSweepNoteUnknown: "Compares every file on both sides. Syncing keeps working while it runs.",
  sweepNow: "Sweep now",

  skipIntro:
    "Anything matching a rule below stays on this computer and is never copied to Proton Drive. Rules are matched against the path inside your sync folder.",
  yourRules: "Your rules",
  hidingTotal: (n, size) => `hiding ${count(n)} files, ${bytes(size)} in total`,
  skippingNow: (n) => `Skipping ${count(n)} files right now`,
  skippingSize: (n, size) => `Skipping ${count(n)} files, ${bytes(size)}`,
  ruleAdded: (date) => `added ${date} · the folder still exists on this computer`,
  // The drawn line without its date. A TOML array of globs carries no per-entry timestamps, so
  // `added 14 Jul` has no possible source — not a missing command, an absent fact.
  ruleFolderHere: "the folder still exists on this computer",
  matchingNothing: "Matching nothing",
  // A rule the walk could not evaluate — a bad glob, or a folder it was refused. `RuleUsage.error`
  // carries the reason and it goes in the mono line beneath, unrewritten. No frame draws it, and
  // `Matching nothing` would be the wrong thing to say: nothing was measured, not nothing matched.
  ruleUnchecked: "Couldn't be checked",
  // The walk is still running. `skip_rule_usage` reads every file in the sync folder, which is
  // seconds on a large one — and for that whole time the rows have no counts. Saying so is the
  // only honest thing here: `Matching nothing` would be a measurement nobody took, over a rule the
  // next line invites you to remove.
  ruleChecking: "Checking…",
  // A rule typed in and not saved. `skip_rule_usage` walked the config on disk, so this rule has no
  // counts — and borrowing a neighbour's or drawing zeros would both say something untrue about how
  // many files it hides.
  ruleNotSaved: "Not saved yet",
  staleRule: "no such folder here any more — safe to remove",
  // `hidingTotal` when the walk could not read everything. `skip_rule_usage` returns
  // `unreadable_directories`/`unreadable_entries` precisely so the tab does not present a floor as
  // a fact (`skip_rules.rs`), and every number on this tab is then a lower bound.
  hidingFloor: (n, size) =>
    `hiding at least ${count(n)} files, ${bytes(size)} — some folders could not be read`,
  remove: "Remove",
  addRulePlaceholder: "Add a rule — e.g. *.psd or scratch/**",
  add: "Add",
  unsyncableNote:
    "Two more files can't be synced no matter what — a socket and a shortcut. Nothing you can change here.",
  seeThem: "See them",
  dotSyncNote: "The app's own .sync folder is always skipped and can't be added here.",

  deletionsTitle: "When a file is deleted",
  deletionsSub: "Deleting on one side would normally delete it on the other. This is how much say you get.",
  askEvery: "Ask me every time",
  recommended: "recommended",
  askEverySub: "Deletions wait in a queue until you approve them. Nothing disappears behind your back.",
  askPermanent: "Only ask about permanent ones",
  askPermanentSub:
    "Deletions that go to Proton's Trash happen automatically. Anything removed from this computer for good still waits for you.",
  askNever: "Never ask",
  askNeverSub:
    "Deleting a file on either side deletes it on the other immediately, including permanently from this computer.",

  saveNote:
    "Saving writes only what you changed. Your comments and anything the app doesn't understand are left alone.",
  discard: "Discard changes",
  save: "Save",
  ruleRemovedCost: (n, size) => `One rule removed — ${count(n)} files, ${bytes(size)} will start syncing.`,

  refusedTitle: "That folder doesn't exist on Proton Drive",
  refusedBody:
    "Nothing was saved — your old settings are still running. Create the folder on Proton Drive first, or pick a different one.",
  // Voice rule 4 again: the daemon's own words, in mono, never rewritten.
  refusedDaemonExample: "remote_root: /Drive/Archive2026 — not found",
  refusedBack: "Go back and fix it",
  refusedCreate: "Create it on Proton Drive",
  // The refusal Phase 1 can actually produce. `write_config` refuses on `ConfigDoc::validate` — a
  // serde/TOML check against `FileConfig` that never contacts Proton Drive — so it cannot know a
  // remote folder is missing and cannot say so (G16, #236). The title therefore names what IS known
  // (the save did not happen) and the body keeps the sentence `08-settings.md` calls the important
  // one, which is true of every refusal whatever caused it.
  refusedTitleUnknown: "That change wasn't saved",
  refusedBodyUnknown: "Nothing was saved — your old settings are still running.",

  // The save landed, and the daemon is still running the old config. There is no config-reload path
  // in the engine — no SIGHUP handler, no watcher (DEVIATIONS §68) — so `Changes here take effect on
  // the next sync` is true only after a restart. Undrawn: no frame draws a settled save.
  savedNote: "Saved. The sync service is still running the old settings until it restarts.",
  restart: "Restart it now",
  restarting: "Restarting the sync service…",
  // Templates, so the daemon's own words go in unrewritten (voice rule 4). Every one of these is a
  // state the bar reports and no frame draws — a control that answered with silence is the failure
  // #140 already recorded once.
  restartFailed: (reason) => `The sync service did not restart — ${reason}`,
  saving: "Saving…",
  sweeping: "Starting a full sweep…",
  sweepFailed: (reason) => `The full sweep didn't start — ${reason}`,
  chooseFailed: (reason) => `The folder picker didn't open — ${reason}`,

  // ------------------------------------------------------------------- Advanced (not drawn) ----
  // `08-settings.md` names six things this tab holds and does not draw it. Four of the six have no
  // key and no command (G17, #237); these are the two that round-trip through `ConfigUpdate`, plus
  // the file itself, which is not a setting but is the answer to "where did this go".
  includeTitle: "Only sync these",
  includeSub:
    "With nothing here, everything syncs except what the rules on What to skip hide. Add a pattern and only matching files sync.",
  includeEmpty: "No patterns — everything syncs.",
  addIncludePlaceholder: "Add a pattern — e.g. work/** or *.md",
  cliTitle: "Where the Proton Drive command lives",
  cliSub: "The app runs this program to reach Proton Drive. A bare name is looked up on PATH.",
  configFileTitle: "The file these settings are written to",
  configFileMissing: "Not created yet — saving writes it.",
  // The config file could not be read — a TOML typo, or a permission. A template, so the reason is
  // the parser's own words (voice rule 4). Undrawn, and it outranks everything else on the screen:
  // every control below it is describing a file nobody could open.
  configUnreadable: (reason) => `These aren't the settings that are running — ${reason}`,
  advancedMissing:
    "Log level, the socket path, the conflict suffix and resetting the index aren't settings the app can write yet.",
};

// --------------------------------------------------------------------------- onboarding ----

/**
 * One install command per package family `gui_core::distro` can return.
 *
 * ⚠ **THE COMMANDS DO NOT WORK, AND THAT IS NOT A BUG IN THIS TABLE.** `9a CLI missing` draws
 * `sudo apt install proton-drive`, and this project's own documentation contradicts it twice —
 * "`proton-drive` is not available in Linux distribution repositories, so it can't be a package
 * dependency", and "The native packages deliberately do **not** declare `proton-drive` as a
 * dependency (it isn't in any distro repo)". There is no distribution where a package manager
 * installs it, so every command here is the drawn artefact rather than an instruction that
 * succeeds.
 *
 * The Debian row is kept verbatim because the deck's job is to hold what the design draws, and
 * dropping it would quietly remove the string from the copy gate. **S7 must not ship a copyable
 * command box from this table** until the design settles what the real instruction is — the
 * `Detected …` sentence is the part of C5 that works today, and the tarball branch is correct for
 * everyone. DEVIATIONS.md §72 carries this, and #218 tracks it.
 */
const CLI_INSTALL_COMMANDS = {
  debian: "sudo apt install proton-drive",
  fedora: "sudo dnf install proton-drive",
  arch: "sudo pacman -S proton-drive",
  suse: "sudo zypper install proton-drive",
  alpine: "sudo apk add proton-drive",
};

export const ONBOARDING = {
  foldersTitle: "Which two folders should match?",
  foldersSub: "One on this computer, one on Proton Drive. From then on they stay identical.",
  chooseLocal: "Choose a different folder…",
  browseRemote: "Browse Proton Drive…",
  emptyIsFine: "A new empty folder is fine — everything on Proton Drive will be brought down into it.",
  signedIn: (email, used, total) => `Signed in as ${email} · ${bytes(used)} of ${bytes(total)} used`,
  skipHint:
    "You can tell it to skip things — screenshots, huge exports, scratch folders — now or any time later in Settings.",
  addSkipRules: "Add skip rules",
  nothingUntilApproved: "Nothing is copied or changed until you approve the plan.",
  seeWhatHappens: "See what will happen",

  reviewTitle: "Nothing gets deleted today",
  reviewSub:
    "The first sync only adds. Files you have go up, files on Proton come down, and anything that exists on both sides in different versions is kept as two copies so you can look at them later.",
  goingUp: "Going up to Proton",
  goingUpSub: "Files that only exist on this computer.",
  comingDown: "Coming down to this computer",
  freeSpace: (need, have) => `Needs ${bytes(need)} free. You have ${bytes(have)}.`,
  alreadyMatch: (n) => `${count(n)} files already match on both sides`,
  leftAlone: "left alone",
  differ: (n) => `${count(n)} ${plural(n, "file differs", "files differ")} on both sides`,
  differSub: "both copies kept — you decide later",
  cannotSync: (n) => `${count(n)} files can't be synced — a socket and two shortcuts`,
  skipped: "skipped",
  nothingDeleted: "Nothing will be deleted",
  eitherSide: "on either side",
  workedOut: (ago, eta) => `worked out ${ago} · ${eta} to finish`,
  seeAllActions: (n) => `See all ${count(n)} actions`,
  back: "Back",
  start: "Start the first sync",

  progressTitle: "Bringing everything together",
  progressSub: (done, total, left) => `${count(done)} of ${count(total)} done · ${left}`,
  sent: (n) => `${count(n)} sent`,
  received: (n) => `${count(n)} received`,
  canClose: "You can close this window — it keeps going in the background.",
  progressFooter: "nothing deleted · 2 conflicts kept as copies",

  doneTitle: "Both sides now match",
  doneSub: (files, size) =>
    `${count(files)} files, ${bytes(size)}. Nothing was deleted, and 2 files are waiting for you to pick a version.`,

  consentTitle: "One thing to agree to before it runs on its own",
  consentBody:
    "From now on, deleting a file on either side deletes it on the other. You'll be asked before each one — and you can change that in Settings — but this is how the two folders stay identical.",
  consentCheckbox: "I understand deletions travel both ways.",
  consentPaused: "Syncing stays paused until you agree.",
  consentStart: "Start syncing",

  // ---- the Phase-1 forms. Each is the drawn sentence with the clause no command can answer taken
  // out, and each is a template so the copy gate keeps checking the drawn original. §79.
  /** `files · 1.4 GB`, or `files` alone — no dry-run field carries a size (#191, #206). */
  sideUnit: (n, size = null) => `${plural(n, "file", "files")}${size ? ` · ${bytes(size)}` : ""}`,
  /** `freeSpace` minus `Needs 38.4 GB free.`, which has no source at any level of the plan (#206). */
  freeSpaceHave: (have) => `You have ${bytes(have)}.`,
  /** `cannotSync` minus the kinds it names — nothing enumerates them (#232). */
  cannotSyncPlain: (n) => `${count(n)} ${plural(n, "file", "files")} can't be synced`,
  /** `workedOut` minus the estimate; no command reports how long a pass will take (#229). */
  workedOutPlain: (ago) => `worked out ${ago}`,
  /** `progressSub` minus `about 17 minutes left` (#229). The fraction is `SyncActivity`'s own. */
  progressDone: (done, total) => `${count(done)} of ${count(total)} done`,
  nothingDeletedShort: "nothing deleted",
  conflictsKept: (n) => `${count(n)} ${plural(n, "conflict", "conflicts")} kept as copies`,
  /** `doneSub` minus its totals (#207), with the conflict count taken from the plan rather than 2. */
  doneSubPhase1: (conflicts) =>
    conflicts
      ? `Nothing was deleted, and ${count(conflicts)} ${plural(conflicts, "file is", "files are")} waiting for you to pick a version.`
      : "Nothing was deleted.",

  cliMissingTitle: "Proton Drive's command line tool isn't installed",

  /**
   * The install instructions, for the distribution `gui_core::distro` detected — or for none.
   *
   * Both halves take the detection result because the frame hard-codes one distribution in each,
   * and a detected distribution had nowhere to go until they did: `Detected Debian` sits inside the
   * body sentence, and the command box says `sudo apt install proton-drive`.
   *
   * `null` is not a failure to handle politely — it is the documented answer. `09-onboarding.md`
   * and `14-behaviour-and-state.md` both say to show the tarball instructions rather than guess a
   * package manager, so an unrecognised machine gets a download and no `$` command that would fail.
   */
  cliMissingBody: (distro) =>
    "This app drives the official tool rather than talking to Proton directly. Install it once and setup will carry on. " +
    (distro
      ? `Detected ${distro.name} — other distributions are in the help.`
      : "We couldn't tell which distribution this is — the download and instructions are in the help."),
  cliInstallCommand: (distro) => CLI_INSTALL_COMMANDS[distro?.id] ?? null,
  copy: "Copy",
  checkAgain: "Check again",
  installHelp: "Installation help",
};

// --------------------------------------------------------------------------------- tray ----

export const TRAY = {
  open: "Open Drive Sync",
  syncNow: "Sync now",
  pause: "Pause syncing",
  resume: "Resume syncing",
  tryAgain: "Try again now",
  closeWindow: "Close window",
  // The two sub-labels 10-tray.md calls the single worst misunderstanding a tray app can cause.
  // They are not decoration and they are not shortenable further.
  closeWindowSub: "keeps syncing",
  quit: "Quit",
  quitSub: "stops syncing",

  /**
   * The two sentences for a reachable daemon that has never synced — a state NO FRAME DRAWS and the
   * deck has no words for, because in the window it is unreachable: `app.js` intercepts `firstRun`
   * with the onboarding takeover before the main screen renders. The tray has no takeover, so it is
   * the one surface that must say something, and the alternative was `Everything is up to date` over
   * a daemon that has never copied a file.
   *
   * Written rather than measured, therefore, and kept as close to what already exists as possible:
   * the v1 tray shipped `Nothing synced yet` as a disabled menu item, and the second line points at
   * the two folders `ONBOARDING.foldersTitle` asks about. DEVIATIONS §82g.
   */
  nothingSyncedYet: "Nothing has synced yet",
  nothingSyncedYetSub: "Open Drive Sync to choose your two folders.",

  unreachableTitle: "Can't reach Proton Drive",
  /**
   * `n == null` DROPS THE SECOND SENTENCE rather than rendering the count, and this is the one place
   * in the deck where that matters most.
   *
   * When the daemon is unreachable there IS no reply, so the pending count is genuinely unknown —
   * and `14-behaviour-and-state.md`'s rule for that is absolute: *"a null summary means unknown, not
   * zero (render em-dashes, never `0`)"*, which `gui-core`'s `DaemonState::Unreachable` doc,
   * `store.select.countersUnknown()` and `format.dash()` all restate. `0 changes are waiting` is a
   * false all-clear at the exact moment the app cannot see anything, and an em-dash mid-sentence
   * (`— changes are waiting`) is not English.
   *
   * So the clause goes, and the sentence that carries the reassurance stays. Same shape as
   * `MAIN.band.deletionSub` dropping a zero clause, and the same rule as every Phase-1 omission on
   * the main screen: omit what is not known, never fill it.
   */
  unreachableBody: (n) =>
    n == null
      ? "Nothing is lost."
      : `Nothing is lost. ${count(n)} ${plural(n, "change is", "changes are")} waiting and will go as soon as it's back.`,
  retrying: (inTime, lastAt) => `retrying in ${inTime} · last reached ${lastAt}`,
};

// ------------------------------------------------------------------------ notifications ----

/**
 * The four banners, the rules sheet and the `notify_policy` cards (S9).
 *
 * `13-copy-deck.md` §Notifications delegates to `11-notifications.md`, so unlike every block above
 * these were read off the frames. `copy-gate.mjs` checks all of them against `11a In situ`,
 * `11a Outage`, `11a Grouped`, `11a Rules` and `11a Settings`, including six templates.
 *
 * Four actions are imported, not retyped: they are the attention band's, the compact panel's and the
 * tray's own words for the same three jobs, and a banner offering `Open Drive Sync` is offering the
 * tray's row.
 *
 * No key here names a destructive act — `11-notifications.md`'s hard rule. `ui/notification.js`
 * enforces it on the builders rather than trusting this list.
 */
export const NOTIFY = {
  /** The notification server's app name. Not `CHROME.productName`; all five frames draw two words. */
  app: "Drive Sync",

  /**
   * `1,204 photos would be deleted from this computer`.
   *
   * The app passes no noun and the queue length as `n`: nothing types a queued deletion, and nothing
   * counts what is under a folder about to go (G8 #208). The deck keeps the frame's form so the gate
   * can still check it — same shape as `SETTINGS.pairLocalNote`.
   */
  deletionTitle: (n, noun = null) =>
    `${count(n)} ${noun ?? plural(n, "file", "files")} would be deleted from this computer`,
  /** The path is separate: it is drawn in mono inside the sentence, and the frame's text breaks there. */
  deletionBodyAfter: " was deleted on Proton Drive. Nothing has happened here yet.",
  deletionKeep: "Keep them",
  deletionReview: MAIN.band.deletionAction,

  conflictTitle: (path) => `You both changed ${path}`,
  conflictBody: "Both versions are safe. Pick one when you have a moment.",
  conflictCompare: MAIN.band.conflictAction,
  later: MAIN.compact.later,

  /** Digits where `MAIN.band.conflictTitle` spells the word — same words, different string. */
  groupedTitle: (n) => `${count(n)} files changed on both sides`,
  /** Versions, not files: two copies per conflict, and the larger number is the reassurance. */
  groupedBody: (versions) =>
    `All ${cardinal(versions, "mid")} versions are safe. This usually means another device was offline for a while.`,
  groupedAction: "Go through them",

  firstSyncTitle: "Both sides now match",
  /** The totals are G7 (#207); the clause goes rather than being filled, as `2a Settled` does. */
  firstSyncBody: (files, size) =>
    files == null || size == null
      ? "First sync finished. Nothing was deleted."
      : `First sync finished — ${count(files)} files, ${bytes(size)}. Nothing was deleted.`,

  outageTitle: "Nothing has synced since yesterday",
  /** `n == null` drops the count clause — unknown is never zero (`TRAY.unreachableBody`'s rule). */
  outageBody: (n) =>
    n == null
      ? "Proton Drive is asking you to sign in again. Nothing is lost."
      : `Proton Drive is asking you to sign in again. ${count(n)} ${plural(n, "change is", "changes are")} waiting — nothing is lost.`,
  /**
   * `11a Outage` draws `Sign in`. Nothing in the command surface signs in — the daemon reuses the
   * CLI's keyring session, so re-authenticating is `proton-drive login` in a terminal. DEVIATIONS
   * §67a settled this for the main screen's hero; `Try again now` is the tray's word for the same act.
   */
  outageRetry: TRAY.tryAgain,
  outageOpen: TRAY.open,

  /**
   * `11a Rules`. Every `why` line differs from `11-notifications.md`'s table — the frame wins
   * (IMPLEMENTATION-PLAN §1.3 rule 2). Do not restore the prose wording.
   */
  rules: {
    /** `— 4` is drawn, not counted. */
    interruptsTitle: "Interrupts you — 4",
    interrupts: [
      {
        title: "Something would be deleted permanently",
        why: "The one event where waiting silently could cost you files you'd never get back.",
      },
      {
        title: "A file changed on both sides",
        why: "Nothing is lost, but you're now editing two versions of the same thing without knowing it.",
      },
      {
        title: "The first sync finished",
        why: "Once, at the end of a long wait you were told to walk away from.",
      },
      {
        title: "Nothing has synced for a day",
        why: "Not a blip — a real outage, wrong password, or full drive. Silence here is dangerous.",
      },
    ],
    silentTitle: "Stays silent — on purpose",
    silent: [
      "every sync pass",
      "every file sent",
      "every file received",
      "folders followed",
      "renames",
      "a single failed pass",
      "retries",
      "scheduled sweeps",
      "skipped files",
      "pause and resume",
      "recoverable deletions",
      "settings saved",
    ],
    /** Three fields because the frame's text breaks around an `<a>`. The trailing space is drawn. */
    activityBefore: "All of it is in ",
    activityLink: CHROME.doors.activity,
    activityAfter:
      ", where you go looking for it. A notification you didn't need is a notification you'll switch off.",
    hardRuleTitle: "Never a button in a banner",
    hardRuleBody:
      "Delete. Discard a version. Approve all. Anything irreversible needs a window where you can see what you're losing — a banner only ever offers the safe direction.",
  },

  /** `11a Settings` — the deletions tab's radio cards, three choices. */
  settings: {
    /** The fifth pill. No frame draws the Settings tab row with it — see `settings.js`'s TABS. */
    tab: "Notifications",
    title: "When to interrupt me",
    sub: "Everything else stays in Activity regardless.",
    /** `default`, not `SETTINGS.recommended` — a different word making a different claim. */
    badge: "default",
    choices: [
      {
        label: "Only when you need me",
        sub: "The four events on the left. Roughly once a week, in a quiet month.",
      },
      {
        label: "Only permanent deletions",
        sub: "The single event that can cost you files. Conflicts wait quietly in the app.",
      },
      {
        label: "Never",
        /** Turning notifications off is not consent: the deletion queue still holds. */
        sub: "The menu bar glyph still changes, and things still wait for you rather than happening on their own.",
      },
    ],
    key: "notify_policy",
  },
};
