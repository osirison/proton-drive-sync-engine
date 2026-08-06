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

import { count, bytes } from "./format.js";

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
  syncing: (n) => `Syncing ${count(n)} changes`,
  syncingSub: (ago, leaving, arriving) =>
    `started ${ago} · ${count(leaving)} leaving, ${count(arriving)} arriving`,
  otherWaiting: (n) => `${count(n)} other changes are waiting on you`,
  paused: "Paused",
  pausedSub: (n, since) =>
    `${count(n)} changes have piled up since ${since}. Nothing will move until you resume.`,

  sideLocal: "This computer",
  sideRemote: "Proton Drive",
  sideRemoteCompact: "Proton",

  syncNow: "Sync now",
  pause: "Pause",
  resume: "Resume syncing",
  queued: "queued",

  footerPair: (local, remote) => `${local} ⇄ ${remote}`,
  footerTotals: (sent, received) => `${bytes(sent)} sent · ${bytes(received)} received today`,

  band: {
    conflictTitle: "One file changed on both sides",
    conflictSub: (path) => `${path} · both copies kept, nothing lost`,
    conflictAction: "Compare",
    deletionTitle: "Two deletions are waiting on you",
    deletionSub: "1 removes from this computer permanently · 1 goes to Proton's Trash",
    deletionAction: "Review",
  },

  compact: {
    upToDate: "Up to date",
    needYou: (n) => `${count(n)} things need you`,
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
  meta: "a plain text file · last agreed 3 hours ago",

  mine: "Your version · this computer",
  mineChange: "You added a line, 5 minutes ago",
  mineDiff: "Yours has buy milk where Proton's has something else, and is otherwise the same.",
  theirs: "Proton's version · from another device",
  theirsChange: "Changed a line and added one, 2 minutes ago",
  theirsDiff: "Proton's has buy oat milk and an extra line at the end.",

  showDiff: "See the exact differences",
  hideDiff: "Hide differences",
  openBoth: "Open both in an editor",

  keepMine: "Keep mine",
  keepMineSub: "Your version goes to Proton Drive. Proton's version is discarded.",
  keepBoth: "Keep both",
  keepBothSub: "Nothing is lost. Proton's copy lands beside yours as todo.proton-cloud.txt.",
  useTheirs: "Use Proton's",
  useTheirsSub: "Proton's version replaces the file on this computer. Yours is discarded.",

  cannotUndo: "Discarding a version can't be undone from here.",
  later: "Decide later",

  diffSummary: "Two lines differ. Everything else in the file matches.",
  diffCounts: "2 lines differ · 3 lines identical",
  absentLine: "not in your version",

  stillWaiting: "Still waiting after this one",
  bothChanged: "both changed it",
  typeConflict: "a folder here, a file there",

  clearedTitle: "Nothing left to decide",
  clearedSub: "You settled 3 files. Two kept both versions, one took Proton's copy.",
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
  cliMissingBody:
    "This app drives the official tool rather than talking to Proton directly. Install it once and setup will carry on. Detected Debian — other distributions are in the help.",
  cliInstallCommand: "sudo apt install proton-drive",
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
  unreachableBody: (n) =>
    `Nothing is lost. ${count(n)} changes are waiting and will go as soon as it's back.`,
  retrying: (inTime, lastAt) => `retrying in ${inTime} · last reached ${lastAt}`,
};
