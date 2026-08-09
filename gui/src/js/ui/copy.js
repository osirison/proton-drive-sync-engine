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
  // `a folder here, a file on Proton Drive` until the queue list was measured — a sentence I wrote
  // from the prose in `04-conflicts.md` rather than off a frame, which is exactly what the copy
  // gate suspected when it refused it as undrawn. It IS drawn: `3a Conflict diff`'s third queue row
  // reads `photos/trip · a folder here, a file there · 3 of 3`. The short form is also the better
  // one — the row already says which side is which by sitting in this app.
  kindFolder: "a folder here, a file there",

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
    if (keptBoth > 0)
      parts.push(`${cardinal(keptBoth).toLowerCase()} kept both ${plural(keptBoth, "version", "versions")}`);
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

export const DELETIONS = {
  title: "Two files are waiting to be deleted",
  sub: "They were deleted on one side. Nothing happens to the other side until you say so — syncing carries on around them.",

  permanent: "Permanent · this computer",
  permanentSub: "Removed straight from disk. Not moved to any trash, and not recoverable from Proton.",
  recoverable: "Recoverable · Proton Drive",
  recoverableSub: "Moved to Proton Drive's Trash. You can restore it there until the trash is emptied.",

  folderConsequence: (n, size) =>
    `Deleting this removes ${count(n)} photos, ${bytes(size)} from this computer, including everything inside it.`,
  travelExplainer:
    "You deleted this on this computer. Deleting it on Proton moves it to Proton Drive's Trash, where you can still get it back.",

  typeToDelete: "To delete it, type DELETE below.",
  delete: "Delete",
  toTrash: "Move to Proton's Trash",
  keepRemote: "Keep it — put it back on Proton Drive",
  keepLocal: "Keep it — bring it back to this computer",
  noExpiry: "Deletions stay here until you decide. Nothing expires.",
  keepBoth: "Keep both files",

  armedTitle: (n) => `Delete ${count(n)} photos from this computer?`,
  armedBody:
    "Everything in photos/2019 — 8.4 GB — is removed from disk. It does not go to your trash, and it is already gone from Proton Drive, so there is nothing to restore it from.",
  armedConfirm: "Delete permanently",
  armedCancel: "Press Esc to cancel.",

  emptyTitle: "Nothing waiting to be deleted",
  emptySub:
    "When a file disappears from one side, it waits here for you instead of vanishing from the other.",

  compact: {
    title: (n) => `${count(n)} files waiting to be deleted`,
    permanent: "1,204 photos gone from this computer, permanently",
    recoverable: "to Proton's Trash — recoverable",
    review: "Review them",
  },
};

// -------------------------------------------------------------------------- plan a sync ----

export const PLAN = {
  title: (n) => `The next sync moves ${count(n)} things`,
  sub: "One of them can't be undone. Everything here is a rehearsal — nothing has changed yet.",
  checkAgain: "Check again",

  leaving: "Leaving this computer",
  arriving: "Arriving from Proton",
  filesAnd: (n, size) => `${count(n)} files, ${bytes(size)}`,
  plusFolder: "Plus one new folder created on Proton Drive to hold them.",
  plusRename: "One file you renamed will be renamed here to match.",

  destructiveTitle: "One file gets deleted for good",
  destructiveBody:
    "archive/old-notes.md is removed from Proton Drive. It's already gone from this computer, so nothing will bring it back.",
  leaveItAlone: "Leave it alone",

  everyAction: "Every action, in order",
  actionSummary: (n) => `${count(n)} actions · 1 conflict kept as both copies`,

  gate: "type DELETE to allow it",
  gateWhy: "Only needed because this plan deletes something.",
  runWithout: "Run it without the deletion",
  run: "Run this sync",

  safeTitle: "Nothing gets deleted",
  safeSub: "Five files move, both sides end up with everything. This plan is safe to run.",
  newFolder: "new folder",
  moved: "moved",
  checkedAgo: (ago) => `Checked ${ago} against both sides.`,

  checkingTitle: "Working out what would change",
  checkingSub: "Comparing both sides. Nothing is being touched.",
  checkingProgress: (done, total) => `${count(done)} of ${count(total)} files`,
  stop: "Stop",
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

  neverSyncedTitle: (n) => `${count(n)} files are never synced`,
  neverSyncedSub:
    "They sit in your folder but aren't copied anywhere. Two match a rule you wrote; two can't be synced at all.",
  showThem: "Show them",

  lastToMove: "Last things to move",
  lastToMoveSub: (n, days) => `${count(n)} files in the last ${days} days`,
  quietIsNormal: "Quiet is normal — most days nothing needs to move.",
  allFiles: (n) => `All ${count(n)} files`,
  passesTab: "Sync passes",
  nothingRecent: "Nothing has moved in the last hour.",

  lookup: {
    safe: "Safely on both sides",
    safeSub: (at) => `Identical here and on Proton Drive since ${at} today.`,
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
    title: (n) => `${count(n)} files are never synced`,
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
    summary: "18 of the last 20 passes finished cleanly. One failed and retried on its own.",
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

  runOne: "Run one now",
  fullSweep: "Full sweep now",
  fullSweepNote:
    "Takes about 4 minutes; syncing keeps working. Last one 2 days ago — nothing was out of step.",
  sweepNow: "Sweep now",

  skipIntro:
    "Anything matching a rule below stays on this computer and is never copied to Proton Drive. Rules are matched against the path inside your sync folder.",
  yourRules: "Your rules",
  hidingTotal: (n, size) => `hiding ${count(n)} files, ${bytes(size)} in total`,
  skippingNow: (n) => `Skipping ${count(n)} files right now`,
  skippingSize: (n, size) => `Skipping ${count(n)} files, ${bytes(size)}`,
  ruleAdded: (date) => `added ${date} · the folder still exists on this computer`,
  matchingNothing: "Matching nothing",
  staleRule: "no such folder here any more — safe to remove",
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
  differ: (n) => `${count(n)} files differ on both sides`,
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
