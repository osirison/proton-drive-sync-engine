// The copy gate (F8) — every fixed string in ui/copy.js appears verbatim somewhere in the drawn
// frames.
//
// The F8 issue frames this as asserting the app's DOM against the deck. That version needs screens,
// and the screens are S1–S11. This one needs neither: it checks the MODULE against the FRAMES, both
// of which exist now, and it catches the same bug class the issue names — a smart quote typed
// straight, a "pending" where the deck says "waiting", a sentence quietly reworded. It is the
// direction that can run today, and when the screens land assert.mjs checks the third side of the
// triangle (app renders what copy.js says).
//
// Only string constants are checked. A template like `Syncing ${n} changes` cannot be compared
// verbatim without inventing the number, and inventing it would assert the fixture's data rather
// than the design's words.

import { readFileSync, readdirSync } from "node:fs";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import * as COPY from "../../src/js/ui/copy.js";

const HERE = dirname(fileURLToPath(import.meta.url));
const FRAMES = join(HERE, "frames");

// Strings the deck carries that are deliberately NOT drawn in any frame, with the reason. Anything
// not on this list must appear, or the gate fails.
const NOT_DRAWN = new Map([
  // NINE TRAY STRINGS WERE EXEMPT HERE AND SHOULD NEVER HAVE BEEN. The reason given was "native tray
  // menu — no DOM", and it was wrong twice over: the tray panel `10-tray.md` specifies is a webview
  // and not a native menu, and — the part that needed no design decision to notice — all nine are
  // drawn, in frames this gate already reads. `Open Drive Sync`, `Sync now`, `Pause syncing`,
  // `Close window`/`keeps syncing` and `Quit`/`stops syncing` are in `10a Settled`; `Resume syncing`
  // is in `10a Paused`; `Try again now` is in `10a Offline`.
  //
  // They passed the moment the exemption was lifted, so nothing was hidden — but nine of the deck's
  // strings had been outside the gate since F8, including the two sub-labels `10-tray.md` calls the
  // single worst misunderstanding a tray app can cause. `NOT_DRAWN` is a `continue` before the
  // lookup, so an entry here is never checked against the frames even to confirm it is absent;
  // an exemption that is wrong stays wrong silently. S8 removed all nine.
  //
  // The two below are the genuine article — a state no frame draws, written rather than measured.
  // `app.js` hides `firstRun` behind the onboarding takeover, so the window never renders it; the
  // tray has no takeover and would otherwise say `Everything is up to date` about a daemon that has
  // never copied a file. See the note in ui/copy.js and DEVIATIONS §82g.
  ["TRAY.nothingSyncedYet", "no frame draws a tray panel for a daemon that has never synced"],
  ["TRAY.nothingSyncedYetSub", "no frame draws a tray panel for a daemon that has never synced"],
  // The deck's Activity section carries this under "Quiet:", and its only frame is `6a Quiet` —
  // one of the two demoted tide-chart Activity frames that IMPLEMENTATION-PLAN §1.2 puts out of
  // scope. So the deck outlived the drawing. Kept in copy.js because 14-behaviour-and-state.md's
  // empty-state table still specifies it ("Activity › files: `Nothing has moved in the last hour.`
  // + flat line"), and S5 will need it. DEVIATIONS.md §49.
  ["ACTIVITY.nothingRecent", "only drawn in `6a Quiet`, which is out of scope"],
  // The label for a failed pass whose error names no cause (#258). `6a Activity passes` draws one
  // failure and it is a connection timeout, so the frame has no occasion to draw this — which is the
  // whole reason the screen shipped claiming every failure was Proton being unreachable.
  [
    "ACTIVITY.passes.failed",
    "the frame's only failed pass is a connection timeout, which keeps the drawn label",
  ],
  // The fifth Settings pill (S9). `11a Settings` is the tab's CONTENT, drawn as a crop with no pill
  // row above it, and `8a Settings` draws the row with the four `08-settings.md` names. So the word
  // exists in no frame while the surface it names is drawn in two. DEVIATIONS §83.
  ["NOTIFY.settings.tab", "no frame draws the Settings pill row with a fifth pill"],
  // S2's meta line names what sort of file this is, and only ONE of the three answers is drawn.
  // `3a Conflict` is a text conflict, so `a plain text file` is in a frame; the other two describe
  // states no `3a` frame draws — a type conflict (the frames put `photos/trip` in the queue list,
  // never open) and a file the pair could not read as text (binary, too large, or vanished, which
  // `ConflictSide` cannot tell apart). The deck has no wording for either, so these are the two
  // sentences S2 wrote rather than measured. DEVIATIONS §74.
  //
  // `kindFolder` NEARLY MOVED OUT OF THIS TABLE, and the near-miss is worth the four lines. The
  // queue list on `3a Conflict diff` does draw `photos/trip · a folder here, a file there`, which
  // looks like the drawn instance this row denies — it is not. That string is `typeConflict`, the
  // list wording. `kindFolder` is the META LINE, the slot that says `a plain text file` under the
  // filename on the card, and no frame opens a type conflict card. Rewriting this one to match the
  // queue's text would have made the gate green by pointing two deck entries at one drawn node,
  // which is the exact failure a verbatim copy gate exists to prevent.
  ["CONFLICTS.kindFolder", "no frame opens a type conflict — the deck has no card copy for one"],
  ["CONFLICTS.kindBinary", "no frame opens a conflict whose pair has no text"],
  // S3's two Phase-1 consequences. Both are sentences the deck does not have because the frame
  // draws the state they replace: `4a Deletions`' permanent card is a FOLDER whose consequence is
  // the subtree aggregate (#208), and no frame draws a permanent FILE deletion at all.
  //
  // They are new sentences rather than the drawn one with the number omitted, which would leave
  // "Deleting this removes from this computer" — see the note on `folderConsequence` in copy.js.
  // The drawn form stays in the deck and stays gated (it is in DRAWN below), so the day #208 lands
  // the wording it has to match is still checked against the frame it came from.
  [
    "DELETIONS.folderConsequenceUnknown",
    "the drawn sentence needs the subtree aggregate (#208) — this is what Phase 1 says instead",
  ],
  ["DELETIONS.fileConsequence", "no frame draws a permanent deletion of a single file"],
  // TRASH MODE (`openspec/changes/trash-local-deletes`). A local deletion now goes to this
  // computer's Trash by default, and the whole `4a` set was drawn against the mode that is no
  // longer the default: every deletions frame draws the local card as `Permanent · this computer` /
  // `Removed straight from disk`, `4a Armed` draws the typed-DELETE gate over it, and `4a Compact`
  // says `gone from this computer, permanently`. Those five sentences are still drawn, still gated,
  // and still correct — they are what the screen says once the user turns permanent deletion back
  // on. The five below are the other mode, which no frame has. See DEVIATIONS §100.
  ["DELETIONS.recoverableLocal", "no frame draws a local deletion in the recoverable column"],
  ["DELETIONS.recoverableLocalSub", "no frame draws a local deletion in the recoverable column"],
  [
    "DELETIONS.recoverableMixedSub",
    "no frame draws a recoverable column holding a Proton-side and a local deletion at once",
  ],
  [
    "DELETIONS.travelExplainerLocal",
    "no frame draws the travel explainer for a local deletion that is recoverable",
  ],
  [
    "DELETIONS.toTrashLocal",
    "no frame draws the local card's button under trash mode — `4a Deletions` draws the permanent form",
  ],
  // S4's failed rehearsal: `14-behaviour-and-state.md`'s empty-and-error table specifies it in prose
  // ("dry run failed → show the daemon string, offer `Check again`") and no frame draws it. The
  // daemon's own message is not copy and never passes through here — it is quoted exactly, in mono
  // (voice rule 4).
  ["PLAN.failedTitle", "no frame draws a failed rehearsal — 14-behaviour-and-state.md specifies it in prose"],
  ["PLAN.failedSub", "no frame draws a failed rehearsal — 14-behaviour-and-state.md specifies it in prose"],
  // S1's failed PASS — the sibling of the two above, and #246's own words for what would close it.
  // The deck has one sentence for a daemon that cannot reach Proton (`TRAY.unreachableTitle`) and
  // it is already spoken by a different state; a pass can fail with Proton perfectly reachable, so
  // reusing it would put a claim in the headline that the quoted string underneath contradicts.
  // The daemon's own message is not copy and never passes through here — voice rule 4.
  //
  // Its sub-line is NOT here: `MAIN.failedSub` is a template, `walk` never collects one, and an
  // exemption for something the gate does not look at would subtract one from a denominator it was
  // never in. The check below is what turned that from a guess into a build failure.
  ["MAIN.failed", "no `2a` frame draws a pass that failed — the deck's error table gives the shape in prose"],
  // S1's stopped SERVICE, and the reason no frame draws it is that the design never had this state:
  // `14-behaviour-and-state.md`'s `unreachable` is *Proton* unreachable ("entered after a failed
  // pass and retry"), while `derive_state`'s `Unreachable` is the CONTROL SOCKET not answering. The
  // deck's one sentence for the former (`TRAY.unreachableTitle`) is a diagnosis of the wrong machine
  // when it is the daemon that is down, which is a reboot away for anyone without the systemd unit.
  //
  // `notRunningSub` IS here and `MAIN.failedSub` is not: that one is a template and `walk` never
  // collects one, this one is a plain string because the state has no reply to count from.
  ["MAIN.notRunning", "no frame draws a daemon that is not running — the design's `unreachable` is Proton's"],
  [
    "MAIN.notRunningSub",
    "no frame draws a daemon that is not running — the design's `unreachable` is Proton's",
  ],
  ["MAIN.starting", "the busy label on a button no frame draws — it is only on screen mid-click"],
  // Its tray row. `10a Offline` draws `Try again now` for the same struck panel, because the design
  // read that panel as an outage; a retry cannot reach a daemon that is not listening. DEVIATIONS §94.
  ["TRAY.start", "no frame draws a tray row for starting the service — `10a Offline` draws the retry"],
  // S4's empty plan — the safe variant with different words. No frame draws a rehearsal that found
  // nothing to do, and it is the likeliest state a user sees.
  [
    "PLAN.nothingTitle",
    "no frame draws a plan with nothing in it — 14-behaviour-and-state.md routes it to the safe variant",
  ],
  [
    "PLAN.nothingSub",
    "no frame draws a plan with nothing in it — 14-behaviour-and-state.md routes it to the safe variant",
  ],
  // S5's four undrawn lookup verdicts. `path_sync_status` answers `synced`, `modified` or
  // `conflict`, reports `tracked:false` for a path the index has never seen, and marks a directory
  // through `entity_kind` — FIVE outcomes, of which `7a File lookup` draws exactly one. The other
  // four are reachable from the first thing anyone types into the field, so they are copy the
  // screen must have and copy no frame can check.
  //
  // `noMatch` is the one sentence here the design DOES specify: `14-behaviour-and-state.md:130`
  // gives it verbatim for `Activity › lookup`, and it is in copy.js for the first time with S5.
  // The other three are S5's wording, and DEVIATIONS §77 records them as such.
  ["ACTIVITY.lookup.noMatch", "14-behaviour-and-state.md specifies it; no frame draws a miss"],
  ["ACTIVITY.lookup.noMatchSub", "no frame draws a miss — nothing in the index matched what was typed"],
  // The chooser. No frame draws it because no frame could: the lookup could only answer about one
  // exact path until `search_files` landed (G21), so "several files match" was unreachable.
  ["ACTIVITY.lookup.chooseTitle", "no frame draws a search that matched more than one file"],
  ["ACTIVITY.lookup.chooseSub", "no frame draws a search that matched more than one file"],
  ["ACTIVITY.lookup.backToMatches", "no frame draws a search that matched more than one file"],
  // `capped` is a TEMPLATE and needs no entry — templates are checked only against the frame that
  // draws them, and none draws this one. The exemption was here for one commit and the gate said so.
  ["ACTIVITY.lookup.changed", "no frame draws a `modified` verdict"],
  ["ACTIVITY.lookup.changedSub", "no frame draws a `modified` verdict"],
  ["ACTIVITY.lookup.conflict", "no frame draws a `conflict` verdict"],
  ["ACTIVITY.lookup.conflictSub", "no frame draws a `conflict` verdict"],
  ["ACTIVITY.lookup.folder", "no frame looks a directory up"],
  ["ACTIVITY.lookup.folderSub", "no frame looks a directory up"],
  ["ACTIVITY.lookup.failed", "no frame draws a lookup whose command failed"],
  ["ACTIVITY.lookup.failedSub", "no frame draws a lookup whose command failed"],
  ["ACTIVITY.lookup.unknownSub", "no frame draws a `sync_status` this build does not recognise"],
  // S6. EIGHTEEN ROWS, AND THE PATTERN IN THEM IS THE SCREEN'S: this is the tab with four tabs and
  // three drawn ones, so a third of what it says has no frame to be checked against. They fall into
  // four groups, and only the first is a design decision Phase 1 made for itself.
  //
  // Note what is NOT here. `pairLocalNoteUnknown`, `ruleFolderHere` and `refusedBodyUnknown` are all
  // Phase-1 forms of a drawn sentence with a clause removed, and the gate finds them inside the
  // longer string — so they are checked verbatim against the frame they came from, which is exactly
  // what should happen to a sentence that is a subset of a drawn one. A new sentence needs a row
  // here; an amputated one does not.
  //
  // 1. The schedule panel's two states no frame draws (#193 built the panel; `SETTINGS.timer` and
  //    `timerSub`, its Phase-1 subject, are gone).
  [
    "SETTINGS.fullScanSubUnknown",
    "G7 (#207): the drawn sub-line counts the sync folder's files, and only `skip_rule_usage` counts them — a full metadata walk the Folders tab deliberately does not run. Same gap as the local helper one panel up, on a different sentence",
  ],
  [
    "SETTINGS.scheduleUnsetKey",
    "#193: no frame draws the panel with no schedule configured, which is what every config written before the key is in",
  ],
  [
    "SETTINGS.scheduleUnset",
    "#193: the sentence beside that key line, for the same reason — and it must not read as a schedule, or the controls' resting positions would claim a value the file does not hold",
  ],
  // 2. A drawn sentence whose every clause needs data nothing returns, so nothing of it survives.
  [
    "SETTINGS.fullSweepNoteUnknown",
    "G24 (#238): `Takes about 4 minutes` and `Last one 2 days ago` both need per-pass data that does not exist",
  ],
  [
    "SETTINGS.refusedTitleUnknown",
    "G22 (#236): `write_config` never contacts Proton Drive, so it cannot say a folder is missing",
  ],
  // 3. States no frame draws, reachable on the first click.
  ["SETTINGS.ruleUnchecked", "no frame draws a rule the walk could not evaluate (`RuleUsage.error`)"],
  ["SETTINGS.ruleChecking", "no frame draws the tab while the local-tree walk is still running"],
  ["SETTINGS.ruleNotSaved", "no frame draws a rule added and not yet saved"],
  // THE ENDINGS OF A SAVE (#320/#335). No frame draws a settled save at all, and since the save
  // restarts the service itself there are five of them — it restarted; it was not running, so there
  // was nothing to restart; the start failed, so nothing is running; it never stopped, so the old
  // daemon is still up; or it could not be told apart. Only the ones with no interpolated reason are
  // fixed strings and so only they are the gate's business; the other three are templates.
  // `SETTINGS.savedNote` — "still running the old settings until it restarts" — was here until the
  // wait it described stopped existing.
  ["SETTINGS.savedRestarted", "no frame draws a settled save"],
  ["SETTINGS.savedNotRunning", "no frame draws a save made while the service is stopped"],
  [
    "SETTINGS.savedUnknownEnding",
    "no frame draws a save whose restart answered with an ending this build cannot name",
  ],
  [
    "SETTINGS.saveInterrupts",
    "no frame draws the bar with a staged change over a running pass — the warning before a save that restarts the service",
  ],
  ["SETTINGS.restart", "the retry after a restart that failed"],
  ["SETTINGS.restarting", "no frame draws a restart in flight"],
  ["SETTINGS.saving", "no frame draws a save in flight"],
  ["SETTINGS.sweeping", "no frame draws a sweep being asked for"],
  // The second panel on the Deletions tab (`openspec/changes/trash-local-deletes`).
  // `8a Deletions tab` draws one panel — the `deletion_policy` guard, `When a file is deleted` down
  // to `deletion_policy · applies to both directions` — and the tab now carries a second one
  // beneath it deciding what a local deletion DOES rather than whether it waits. Two settings, one
  // tab, and no frame drawn with both. The guard panel's own six strings stay gated against that
  // frame. See DEVIATIONS §100.
  // The two policy sub-lines said truthfully under trash mode. `8a Deletions tab` draws the
  // PERMANENT-mode pair (still in the deck, still gated against that frame); these are the same two
  // cards under the other mode, chosen by `policyCopyFor`, and no frame draws that tab in it.
  ["SETTINGS.askPermanentTrashSub", 'no frame draws the policy cards under `local_delete_mode = "trash"`'],
  ["SETTINGS.askNeverTrashSub", 'no frame draws the policy cards under `local_delete_mode = "trash"`'],
  ["SETTINGS.disposalTitle", "no frame draws the disposal panel — `8a Deletions tab` predates it"],
  ["SETTINGS.disposalSub", "no frame draws the disposal panel — `8a Deletions tab` predates it"],
  ["SETTINGS.disposalTrash", "no frame draws the disposal panel — `8a Deletions tab` predates it"],
  ["SETTINGS.disposalTrashSub", "no frame draws the disposal panel — `8a Deletions tab` predates it"],
  ["SETTINGS.disposalPermanent", "no frame draws the disposal panel — `8a Deletions tab` predates it"],
  ["SETTINGS.disposalPermanentSub", "no frame draws the disposal panel — `8a Deletions tab` predates it"],
  // 4. The Advanced tab, which `08-settings.md` specifies in prose and draws nowhere.
  ["SETTINGS.includeTitle", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.includeSub", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.includeEmpty", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.addIncludePlaceholder", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.cliTitle", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.cliSub", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.configFileTitle", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.configFileMissing", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.logTitle", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.logSub", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.socketTitle", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.socketSub", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.socketPlaceholder", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.suffixTitle", "the Advanced tab is not drawn in any frame"],
  ["SETTINGS.suffixSub", "the Advanced tab is not drawn in any frame"],
  [
    "SETTINGS.advancedMissing",
    "G23 (#237): the one remaining gap — resetting the index is a command, not a setting",
  ],
  // The local kinds no frame happens to draw, now TWO FORMS PER KIND (#315): `cannotKind`'s values
  // became `{ one, many }` so `kindsPhrase` can count them, and the deck grew a string per kind for
  // the plural. `9a Review` draws `a socket and two shortcuts` and `8a Skip rules` draws `a socket
  // and a shortcut`, which between them put four of the twelve in a frame — `local_socket.one`,
  // `local_symlink.one` and `local_symlink.many` (inside `two shortcuts`); every other string here
  // is a kind or a multiplicity no drawing has an occasion for.
  //
  // They are written rather than measured for the reason S5's undrawn verdicts are: the daemon can
  // report any of them, and improvising a noun at the call site is how a screen ends up saying two
  // different things about one kind. A plural is not optional either — the phrase counts, so a kind
  // with no plural would be unrenderable the moment two of them turn up.
  [
    "ACTIVITY.neverSyncedDialog.cannotKind.local_socket.many",
    "one socket is drawn, two are not — `9a Review`'s only plural is its shortcuts",
  ],
  [
    "ACTIVITY.neverSyncedDialog.cannotKind.local_fifo.one",
    "the frames draw a socket and a symlink; the daemon can report four more kinds and this is one",
  ],
  ["ACTIVITY.neverSyncedDialog.cannotKind.local_fifo.many", "and its plural"],
  [
    "ACTIVITY.neverSyncedDialog.cannotKind.local_device.one",
    "likewise — a block or character device under the sync folder",
  ],
  ["ACTIVITY.neverSyncedDialog.cannotKind.local_device.many", "and its plural"],
  [
    "ACTIVITY.neverSyncedDialog.cannotKind.local_special_file.one",
    "likewise — the scanner's catch-all for an entry that is neither a regular file nor a directory",
  ],
  ["ACTIVITY.neverSyncedDialog.cannotKind.local_special_file.many", "and its plural"],
  [
    "ACTIVITY.neverSyncedDialog.cannotKind.unrepresentable_path.one",
    "likewise — a name that is not valid UTF-8, so the remote listing could never name it (#270)",
  ],
  ["ACTIVITY.neverSyncedDialog.cannotKind.unrepresentable_path.many", "and its plural"],
  // S7's two sub-screens (#244). The deck draws `Add skip rules` and `See all 471 actions` and no
  // destination for either, so the two headings and the empty-list line are written rather than
  // measured. Everything else on both screens is borrowed from a surface the deck DOES draw, which
  // is why only three entries land here.
  ["ONBOARDING.skipTitle", "no frame draws the skip editor the takeover's own button opens"],
  ["ONBOARDING.noSkipRules", "no frame draws that editor with an empty list"],
  ["ONBOARDING.actionsTitle", "no frame draws the action list `See all N actions` opens"],
]);

/**
 * Templates NO `DRAWN` ROW CAN COVER, with the reason (#372).
 *
 * The sibling of `NOT_DRAWN`, for the other half of the deck. An entry here is a claim that this
 * template exists and cannot be rendered against a frame — because nothing calls it, because no
 * frame draws the state it belongs to, or because the app cannot produce the string the frame
 * contains even though the words are there.
 *
 * THAT LAST ONE IS WHY "the frame contains the words" IS NOT THE TEST. `SETTINGS.monthEdgeNote`
 * renders exactly what `8a Schedule monthly` draws when passed 15 — and the app shows that note only
 * for 29, 30 and 31, because every month has a 15th (§104b). A `DRAWN` row would assert a string the
 * built screen can never produce, which is a gate enforcing a fiction rather than a gap.
 *
 * Populated from a per-template investigation rather than from reading the deck: #250's census was
 * sorted twice by reasoning about files instead of looking, and roughly half its reasons were false
 * (DEVIATIONS §106a). Every reason below names a call site or a frame.
 */
/**
 * Arguments the `absent` fragments are proved reachable at. Not a rendering the gate asserts —
 * nothing here is compared against a frame — only enough shapes to show a fragment is the
 * template's own text: these take counts, paths, error strings, an optional flag, and a distro
 * object. A fragment reachable at none of them is rejected above.
 */
const UNGATED_PROBE_ARGS = [
  ["photos/2019", true],
  ["photos/2019", false],
  [1, "file"],
  [2, "folder"],
  [1],
  [2],
  [0],
  ["a reason"],
  [{ name: "Debian" }],
  [undefined],
  [1, 1],
  ["a", "b"],
  [],
];

const UNGATED_TEMPLATES = new Map([
  [
    "ACTIVITY.lookup.capped",
    {
      why:
        "The capped multi-match list, and no frame draws a search that matched more than one file \u2014 " +
        "`7a File lookup` draws `1 match` and goes straight to the verdict panel. Its three constant " +
        "siblings on the same sub-screen (`chooseTitle`, `chooseSub`, `backToMatches`) are already " +
        "exempt for exactly this reason at copy-gate.mjs:168-170; this is the template the same " +
        "exemption cannot hold.",
      absent: "— narrow the search.",
    },
  ],
  [
    "CONFLICTS.edited",
    {
      why:
        "It takes an EPOCH, not a formatted time, and renders it through `format.js`'s `clock()`, " +
        "which uses the machine's own timezone \u2014 so no fixed argument produces the drawn `edited " +
        "14:38` on every runner. format.js:168's own doc calls it \"the one formatter whose output a " +
        "fixture may not derive\". Already a recorded decision: copy-gate.mjs's S2 block says `edited " +
        "14:38` is deliberately absent from DRAWN for this reason.",
      absent: null,
    },
  ],
  [
    "DELETIONS.armedBodyFile",
    {
      why:
        "`4a Armed` arms a FOLDER, so its body is `armedBody` \u2014 `Everything in photos/2019 \u2014 8.4 GB \u2014 " +
        "is removed from disk. \u2026` \u2014 and deletions.js:602 reaches this template only when `entity_kind " +
        '!== "directory"`. No frame arms a file. The only argument that reproduces frame text is ' +
        '`"photos/2019 \u2014 8.4 GB \u2014"`, which is not a path, so a passing DRAWN row would assert the ' +
        "same kind of fiction as `monthEdgeNote`.",
      absent: null,
    },
  ],
  [
    "MAIN.andMore",
    {
      why:
        "The transfer-list overflow row, and no frame draws a list long enough to overflow: `2a " +
        "Syncing` draws three transfer rows and `5a Plan` nine, with no `+n more` node in any of the " +
        "51 frames. All three call sites are guarded `hidden > 0`, so the app can produce it; nothing " +
        "drew it.",
      absent: null,
    },
  ],
  [
    "MAIN.batchFiles",
    {
      why:
        "The chip on a batched-download row, and no frame draws one \u2014 `2a Syncing`'s three transfer " +
        "rows draw `1.2 MB`, `queued` and `2.4 MB`. `batchFiles(12480)` renders `12,480 files`, which " +
        "ten frames contain in a different slot (the settled sub-line's index total, `2a Compact " +
        "settled`'s panel total, `5a Checking`'s progress, `8a Settings`' pair note), so a DRAWN row " +
        "would go green by pointing the deck's chip at another sentence's words \u2014 the " +
        "`CONFLICTS.kindFolder` near-miss this gate exists to prevent.",
      absent: null,
    },
  ],
  [
    "MAIN.failedSub",
    {
      why:
        "No frame draws a failed pass; its headline `MAIN.failed` is already exempt for that reason " +
        "at copy-gate.mjs:127, and this is the template sibling `NOT_DRAWN` cannot hold. The counted " +
        "branch matches nothing. DO NOT gate the zero branch: `failedSub(0)` renders the bare " +
        "`Nothing is lost.`, which is inside `10a Offline` and `3a Conflict`, so a DRAWN row at `[0]` " +
        "would pass while asserting the failed-pass sub-line against the tray's outage body and the " +
        "conflict card's keep-both sub.",
      absent: "waiting and will go on the next try.",
    },
  ],
  [
    "ONBOARDING.cliMissingBody",
    {
      why:
        "The app renders it, but `9a CLI missing` draws the pre-#218 sentence \u2014 `This app drives the " +
        "official tool rather than talking to Proton directly. Install it once and setup will carry " +
        "on. Detected Debian \u2014 other distributions are in the help.` \u2014 and neither branch of the " +
        "current text appears in any of the 51 frames. #218 settled the drawn install command as true " +
        "for no distribution, so the deck deliberately no longer holds a string that matches the " +
        "frame.",
      absent: "and no Linux distribution packages it",
    },
  ],
  [
    "PLAN.checkingProgressBare",
    {
      why:
        "`5a Checking` draws the denominator form `8,431 of 12,480 files` \u2014 that is " +
        "`checkingProgress`, already in DRAWN. This is the line for a run with nothing to count " +
        "against (plan.js:972-978: `total` absent or 0, e.g. a first run's empty index) and no frame " +
        "draws that run. A DRAWN row here would go GREEN AND BE FALSE: `checkingProgressBare(12480)` " +
        "is `12,480 files`, a suffix of that same node and the entire text of an unrelated node in " +
        "`2a Compact settled` \u2014 two deck entries on one drawn node, the `kindFolder` near-miss above.",
      absent: null,
    },
  ],
  [
    "PLAN.destructiveLocal",
    {
      why:
        "`5a Plan`'s band holds one destructive row and it is a `remote_delete`, so the frame draws " +
        "`destructiveRemote`; nothing draws the local mirror. NOT dead copy \u2014 plan.js:529-531 picks " +
        "it as a function REFERENCE in a ternary and calls it at 535, so a `destructiveLocal(` grep " +
        "finds no call site.",
      absent:
        "is removed from this computer. It's already gone from Proton Drive, so nothing will bring it back.",
    },
  ],
  [
    "PLAN.destructiveLocalTrash",
    {
      why:
        'The same row under `local_delete_mode = "trash"`, which is the daemon\'s default \u2014 reachable, ' +
        "and drawn nowhere: the `4a`/`5a` set was captured against permanent deletion, and `moves to " +
        "this computer's Trash` is in no frame. Selected by reference at plan.js:529-530, so a " +
        "`destructiveLocalTrash(` grep is empty.",
      absent:
        "moves to this computer's Trash. It's already gone from Proton Drive, so your file manager is where it comes back from.",
    },
  ],
  [
    "PLAN.destructiveMany",
    {
      why:
        "The band's note for two or more gated rows (plan.js:505, the branch taken when `one` is " +
        "null). `5a Plan` draws exactly one \u2014 `One file gets deleted for good` \u2014 so it renders " +
        "`consequence` instead, and neither `are removed for good` nor the plural title `get deleted " +
        "for good` appears in any frame.",
      absent: "are removed for good. Nothing will bring them back.",
    },
  ],
  [
    "PLAN.plusFolderHere",
    {
      why:
        "The Arriving side's sentence \u2014 plan.js:416 picks it when the new folder is created on this " +
        "computer. `5a Plan` draws only the mirror, `plusFolder`: `Plus one new folder created on " +
        "Proton Drive to hold them.` The words `created on this computer to hold them` are in none of " +
        "the 51 frames.",
      absent: "created on this computer to hold them.",
    },
  ],
  [
    "PLAN.plusRenameThere",
    {
      why:
        "The Leaving side's rename sentence \u2014 plan.js:417 picks it when the rename is applied on " +
        "Proton Drive. `5a Plan` draws only the mirror, `plusRename`: `One file you renamed will be " +
        "renamed here to match.` `renamed on Proton Drive to match` is in no frame.",
      absent: "you renamed will be renamed on Proton Drive to match.",
    },
  ],
  [
    "SETTINGS.chooseFailed",
    {
      why:
        "The notice for a folder picker that could not OPEN, as distinct from one dismissed " +
        "(app.js:2966-2972). `8a Settings` draws `Choose\u2026` with no notice line, and `The folder " +
        "picker didn't open` is in no frame; the reason interpolated is the picker's own error.",
      absent: "The folder picker didn't open —",
    },
  ],
  [
    "SETTINGS.configUnreadable",
    {
      why:
        "Rendered above EVERY tab when `read_config` rejected (settings.js:1444-1449), and all three " +
        "drawn Settings frames \u2014 `8a Settings`, `8a Skip rules`, `8a Deletions tab` \u2014 draw the screen " +
        "without it. `These aren't the settings that are running` is in no frame, and what follows " +
        "the dash is the parser's own message.",
      absent: "These aren't the settings that are running —",
    },
  ],
  [
    "SETTINGS.hidingFloor",
    {
      why:
        "The hedged total, picked at settings.js:246 when `skip_rule_usage` reports an unreadable " +
        "directory or entry. No frame has one, so `8a Skip rules` draws `hidingTotal` \u2014 `hiding 4 " +
        "files, 3.1 GB in total`. `hiding at least` and `some folders could not be read` are in no " +
        "frame.",
      absent: "some folders could not be read",
    },
  ],
  [
    "SETTINGS.monthEdgeNote",
    {
      why:
        "`8a Schedule monthly` draws `Months without a 15th are skipped to the last day.` and the app " +
        "can never say it: settings.js:704 shows the note only for `day > 28`, because every month " +
        "has a 15th (DEVIATIONS \u00a7104b). The three strings it CAN produce \u2014 29th, 30th, 31st \u2014 are in " +
        "no frame, so neither direction yields a row, and a DRAWN row at 15 would assert a string the " +
        "built screen cannot render.",
      absent: null,
    },
  ],
  [
    "SETTINGS.savedNothingRunning",
    {
      why:
        "No frame draws a settled save at all \u2014 `Saved` and `sync service` occur in none of the 51 " +
        "frames, and `8a Save refused` draws a REFUSED save (`Nothing was saved \u2014 your old settings " +
        "are still running.`). This is the `not_started` ending, where the stop succeeded and the " +
        "start did not. Same absence its constant siblings `savedRestarted`/`savedNotRunning` record " +
        "in NOT_DRAWN.",
      absent: "Saved, but the sync service could not be started —",
    },
  ],
  [
    "SETTINGS.savedOldSettings",
    {
      why:
        "The `never_stopped` ending \u2014 the daemon kept answering past the stop timeout and is still up " +
        "on the settings it started with. No frame draws a settled save: `Saved` and `sync service` " +
        "occur in none of the 51 frames, and `8a Save refused` draws a refused save instead.",
      absent: "Saved, but the sync service did not restart —",
    },
  ],
  [
    "SETTINGS.savedUnknownState",
    {
      why:
        "The `undetermined` ending \u2014 nobody could tell whether the service is running, so nothing was " +
        "restarted. No frame draws a settled save: `Saved` and `sync service` occur in none of the 51 " +
        "frames.",
      absent: "Saved, but the app couldn't tell whether the sync service is running —",
    },
  ],
  [
    "SETTINGS.sweepFailed",
    {
      why:
        "The Settings notice for a `Sweep now` that did not start \u2014 `resync` answering with an error " +
        "inside its payload, or the call rejecting (app.js:2987/2989). `8a Settings` draws the sweep " +
        "panel with no notice line, `The full sweep didn't start` is in no frame, and the clause " +
        "after the dash is the daemon's own words, so there is nothing fixed to match.",
      absent: "The full sweep didn't start —",
    },
  ],
]);

/**
 * Templates, WITH the arguments the frame draws them at.
 *
 * The header's rule — "a template cannot be compared verbatim without inventing the number" — is
 * right about inventing and wrong about the number, and S1 is where the difference started to cost
 * something. `2a Needs you`'s band renders from live counts, so `One file changed on both sides` had
 * to become `conflictTitle(n)`; as a constant it was gated, and as a template it silently left the
 * gate. Three of the deck's sentences would have gone quiet in the commit that first drew them.
 *
 * The argument is not invented here — it is READ OFF the frame, exactly as every fixture value is:
 * `2a Needs you` draws one conflict and two deletions, `2a Syncing` draws three changes. So the
 * rendered string is ground truth on both sides and the comparison is the same one every constant
 * gets. What stays out is a template no frame draws (`MAIN.settledSub` needs a relative time, and
 * `since()` against a real clock is exactly the input a gate may not depend on).
 *
 * A path here that is not a function, or that renders a string no frame contains, fails the build.
 */
const DRAWN = [
  ["CHROME.chips.waiting", [3], "2a Needs you"],
  ["CHROME.chips.step", [1], "9a Folders"],
  ["MAIN.syncing", [3], "2a Syncing"],
  ["MAIN.otherWaiting", [3], "2a Needs you"],
  // Drawn on a NOTIFICATION rather than on the main screen: `11a Outage` is the only place the
  // design writes this sentence, and S1's sign-in hero quotes it because there is no second one.
  ["MAIN.authExpiredSub", [61], "11a Outage"],
  ["MAIN.band.conflictTitle", [1], "2a Needs you"],
  ["MAIN.band.conflictSub", ["notes/todo.txt"], "2a Needs you"],
  ["MAIN.band.deletionTitle", [2], "2a Needs you"],
  ["MAIN.band.deletionSub", [1, 1], "2a Needs you"],
  ["MAIN.compact.needYou", [3], "2a Compact needs you"],
  // The C-item templates (C3, C5). Each was a constant until the capability that varies it landed,
  // and each is listed here in the same commit — a template that leaves this table leaves the gate
  // silently, which is the regression the paragraph above records from S1.
  //
  // The arguments are read off the frames the same way every other row's are: `3a Conflict`'s two
  // cards are a one-line change where Proton's also gained a line at the end, and `3a Conflict
  // diff` counts two differing lines against three identical ones.
  [
    "CONFLICTS.versionDiff",
    ["mine", { quoted: "buy milk", extraAtEnd: 0, otherwiseSame: true }],
    "3a Conflict",
  ],
  [
    "CONFLICTS.versionDiff",
    ["theirs", { quoted: "buy oat milk", extraAtEnd: 1, otherwiseSame: false }],
    "3a Conflict",
  ],
  ["CONFLICTS.diffSummary", [2], "3a Conflict diff"],
  ["CONFLICTS.diffCounts", [2, 3], "3a Conflict diff"],
  // S2. The metadata row and the meta line are counted off the pair, so their numbers are the
  // frame's own: `41 bytes` / `4 lines` on the left card, `a plain text file` under the filename.
  // `edited 14:38` is deliberately absent — it renders from an mtime through a local clock, so it
  // moves with the timezone and the hour of the run, which is the one input a gate may not depend
  // on (`fixtures/conflicts.js` names it; DEVIATIONS §74).
  ["CONFLICTS.meta", ["a plain text file"], "3a Conflict"],
  ["CONFLICTS.lineCount", [4], "3a Conflict"],
  ["CONFLICTS.keepBothSub", ["todo.proton-cloud.txt"], "3a Conflict"],
  // The placeholder is drawn on the LEFT only, because the drawn pair puts the extra line on
  // Proton's side. The mirror — `not in Proton's version` — is reachable the moment yours has a
  // line Proton's does not, and no frame draws it; it is checked by `diff.test.js` instead.
  ["CONFLICTS.absentLine", ["mine"], "3a Conflict diff"],
  ["CONFLICTS.clearedSub", [{ total: 3, keptBoth: 2, tookProton: 1 }], "3a Conflicts cleared"],
  // `ONBOARDING.cliMissingBody` and `.cliInstallCommand` WERE HERE, checking the deck against the
  // frame's own `sudo apt install proton-drive` text. Retired with it (#218, DEVIATIONS §102): the
  // decision this closes settles the drawn command as permanently wrong, so the deck no longer
  // holds a string that matches it — `cliInstallCommand` is deleted outright, and `cliMissingBody`
  // now names the manual install path instead of the frame's "other distributions are in the
  // help", which is nowhere in the frozen `9a CLI missing` capture to match against.
  // Was a template with ONE argument and a hard-coded tail — `${n} files can't be synced — a socket
  // and two shortcuts` — so it rendered the frame's kinds whatever the machine held, and the app
  // drew a different, shorter sentence (`cannotSyncPlain`) because it could not produce them. #315
  // gave it the kinds as an argument, which is what puts it in this table: the arguments below are
  // read off `9a Review` exactly as every other row's are, and the app now renders this same
  // sentence from `cannot_sync`. The fifth time a sentence has had to land here in the commit that
  // made it a template.
  ["ONBOARDING.cannotSync", [3, "a socket and two shortcuts"], "9a Review"],
  // S3. Nine rows, and SIX OF THEM ARE STRINGS THAT WERE ALREADY DRAWN AND NEVER CHECKED — the
  // deck's own Deletions section lists the four facts (`deleted on Proton 22m ago`, `last opened
  // Mar 2024`, `deleted here 6m ago`, `last edited Jan 2026`) and F7 left them out of copy.js
  // entirely. Nothing caught it because this gate reads copy.js and the frames; a sentence missing
  // from the MODULE is invisible to it. `title` and `armedBody` were constants and are templates
  // now (a live queue has a count in the first and a path in the second), which is the transition
  // the note above records as the way a sentence leaves the gate silently.
  //
  // Every argument is read off the frame, as ever. Two of them name numbers Phase 1 cannot produce
  // (#208) — that is fine and is the point: this table asserts that the DECK still says what the
  // FRAME draws, which stays true and stays worth checking while the app draws the fallback.
  // S2's two "what happened" sentences (#217/#347). They were CONSTANTS this table checked and are
  // templates now, which is how a checked string leaves this gate silently (#372) — so they are
  // declared here at the arguments that reproduce what the frame draws.
  //
  // The APP draws different verbs on this frame, deliberately: solving `3a Conflict diff`'s own
  // numbers for an ancestor gives `you changed a line, Proton added one`, while the drawn cards say
  // `You added a line` / `Changed a line and added one`, which together force the ancestor to equal
  // your copy — a state that is not a conflict at all. DEVIATIONS §105. These rows assert the DECK
  // can still say what the FRAME draws, which is the same claim `DELETIONS.folderConsequence` makes
  // while the app draws its fallback.
  ["CONFLICTS.happened", ["mine", { added: 1, changed: 0, removed: 0 }], "3a Conflict"],
  ["CONFLICTS.happened", ["theirs", { added: 1, changed: 1, removed: 0 }], "3a Conflict"],
  ["CONFLICTS.happenedAt", ["You added a line", "5 minutes ago"], "3a Conflict"],
  ["DELETIONS.title", [2], "4a Deletions"],
  ["DELETIONS.folderConsequence", ["1,204 photos, 8.4 GB"], "4a Deletions"],
  ["DELETIONS.deletedOnProton", ["22m ago"], "4a Deletions"],
  ["DELETIONS.deletedHere", ["6m ago"], "4a Deletions"],
  ["DELETIONS.lastOpened", ["Mar 2024"], "4a Deletions"],
  ["DELETIONS.lastEdited", ["Jan 2026"], "4a Deletions"],
  ["DELETIONS.armedTitle", ["1,204 photos"], "4a Armed"],
  ["DELETIONS.armedBody", ["photos/2019", "8.4 GB"], "4a Armed"],
  ["DELETIONS.compact.title", [2], "4a Compact"],
  // S4. Eleven rows, eight of which were constants until this commit: a live plan has its own
  // counts and paths, so every sentence on this screen that names one became a template.
  //
  // Two sentences here are in neither table because they are templates no frame draws:
  // `PLAN.destructiveLocal` (the mirror of the drawn sentence, for a deletion applied here —
  // `05-deletions.md` builds its two columns on that same mirror) and `PLAN.destructiveMany`.
  // `NOT_DRAWN` only reaches constants, so a template no frame renders has nowhere to be declared —
  // a hole in this gate rather than in the deck. gui/test/plan.test.js pins both.
  ["PLAN.title", [9], "5a Plan"],
  ["PLAN.sub", [1], "5a Plan"],
  ["PLAN.sideUnit", [3, "4.1 MB"], "5a Plan"],
  ["PLAN.plusFolder", [1], "5a Plan"],
  ["PLAN.plusRename", [1], "5a Plan"],
  ["PLAN.destructiveTitle", [1, "file"], "5a Plan"],
  ["PLAN.destructiveRemote", ["archive/old-notes.md", false], "5a Plan"],
  ["PLAN.actionSummary", [9, 1], "5a Plan"],
  ["PLAN.safeSub", [5], "5a Plan safe"],
  ["PLAN.checkedAgo", ["40 seconds ago"], "5a Plan safe"],
  // Drawn, and not rendered by the app: neither half of it has a source (G9 #209, G7 #207), so S4
  // omits the line whole. The deck still has to say what the frame draws — same shape as
  // `DELETIONS.folderConsequence`, which is gated at a number Phase 1 cannot produce either.
  ["PLAN.checkingProgress", [8431, 12480], "5a Checking"],
  // S5. SEVENTEEN ROWS, AND FIFTEEN OF THEM WERE NEVER CHECKED BY ANYTHING — the whole ACTIVITY
  // block's templates were absent from this table until now, so every counted sentence on the
  // busiest screen in the app was unasserted while its constants were green.
  //
  // Two more became templates in this commit and had to land here in the same one: `passes.summary`
  // (every number in it is live — `status_history` holds up to 20 entries and clean-vs-failed is
  // `last_error == null`) and `neverSyncedSub` (the two group counts). That is the transition the
  // note above records as the way a sentence leaves the gate silently, and this is the third time
  // it has come up.
  //
  // `neverSyncedTitle` and `neverSyncedDialog.title` RENDER THE SAME STRING from two deck entries
  // pointed at two different frames — the band says it at 13.5px and the dialog at 18px, over
  // deliberately different sub-lines. The label on each row is load-bearing: swapping them would
  // still pass, and would stop proving that both frames say it.
  ["ACTIVITY.quietSub", ["14:32", "2 minutes ago"], "7a Activity quiet"],
  ["ACTIVITY.watched", ["2m ago"], "7a Activity quiet"],
  ["ACTIVITY.nextCheck", ["4m"], "7a Activity quiet"],
  ["ACTIVITY.neverSyncedTitle", [4], "7a Activity quiet"],
  ["ACTIVITY.neverSyncedSub", [2, 2], "7a Activity quiet"],
  ["ACTIVITY.lastToMoveSub", [7, 3], "7a Activity quiet"],
  ["ACTIVITY.allFiles", [7], "7a Activity quiet"],
  ["ACTIVITY.matches", [1], "7a File lookup"],
  ["ACTIVITY.lookup.safeSub", ["14:32"], "7a File lookup"],
  ["ACTIVITY.lookup.linked", ["4c8f…9a21"], "7a File lookup"],
  ["ACTIVITY.lookup.pendingSub", ["8 seconds ago", 2800000], "7a File pending"],
  ["ACTIVITY.neverSyncedDialog.title", [4], "7a Never synced"],
  ["ACTIVITY.neverSyncedDialog.ruleSub", ["*.tmp"], "7a Never synced"],
  ["ACTIVITY.passes.summary", [18, 20, 1, true], "6a Activity passes"],
  ["ACTIVITY.passes.chartSub", ["12:45"], "6a Activity passes"],
  ["ACTIVITY.passes.mostRecent", ["14:32"], "6a Activity passes"],
  ["ACTIVITY.passes.retried", ["14:17"], "6a Activity passes"],
  // S6. SEVEN TEMPLATES, NONE OF WHICH WAS CHECKED BY ANYTHING until the screen that draws them was
  // built — the same hole the ACTIVITY block above records, one screen later. Two of them
  // (`pairLocalNote`, `fullScanSub`) are gated here and NOT rendered by the app: their counts are
  // G7 (#207), so the deck still has to say what the frame draws while the screen draws the half of
  // it that has a source. That is the shape `PLAN.checkingProgress` already has.
  ["SETTINGS.pairLocalNote", [12480, 41_200_000_000], "8a Settings"],
  ["SETTINGS.fullScanSub", [12480], "8a Settings"],
  ["SETTINGS.hidingTotal", [4, 3_102_940_000], "8a Skip rules"],
  ["SETTINGS.skippingNow", [2], "8a Skip rules"],
  ["SETTINGS.skippingSize", [2, 3_100_000_000], "8a Skip rules"],
  ["SETTINGS.ruleAdded", ["14 Jul"], "8a Skip rules"],
  ["SETTINGS.ruleRemovedCost", [2, 3_100_000_000], "8a Skip rules"],
  // Was a constant until #232 gave it a count and a list of kinds. Both arguments are read off the
  // frame: `Two more files … a socket and a shortcut`. A sentence that becomes a template leaves
  // this gate silently unless it lands here in the same commit — S1 wrote that rule, and this is
  // the fourth time it has come up.
  ["SETTINGS.unsyncableNote", [2, "a socket and a shortcut"], "8a Skip rules"],
  // S9. The banner sentences. Five of the six are rendered by the app at live values and gated here
  // at the frame's; `deletionTitle` is the sixth and is the `pairLocalNote` shape — the noun is
  // passed to reproduce what the frame draws, and the app never passes one (G8 #208: nothing types
  // a queued deletion or counts what is under it).
  //
  // THREE OF THESE FRAMES ARE NOT WINDOWS. `11a In situ` is a specimen whose product is the three
  // banners inside it, and `11a Outage`/`11a Grouped` are banners the desktop sizes. The gate reads
  // frame TEXT, so it does not care — which is worth saying because it is the only gate in the set
  // that covers the three banners without needing them mounted anywhere.
  ["NOTIFY.deletionTitle", [1204, "photos"], "11a In situ"],
  ["NOTIFY.conflictTitle", ["notes/todo.txt"], "11a In situ"],
  ["NOTIFY.firstSyncBody", [12480, 41_200_000_000], "11a In situ"],
  ["NOTIFY.groupedTitle", [5], "11a Grouped"],
  ["NOTIFY.groupedBody", [10], "11a Grouped"],
  ["NOTIFY.outageBody", [61], "11a Outage"],

  // #372: 25 templates that were in neither table. Each row's arguments are read off the frame
  // named beside it, and each was verified to land on the node that draws that sentence rather
  // than on a coincidence — six match as a CLAUSE of a longer node (`MAIN.settledSubTime` is the
  // opening of `2a Settled`'s sub-line, whose full form is `MAIN.settledSub` two rows up), which
  // is the app drawing a shorter version of the drawn sentence, not a near-miss.
  ["CONFLICTS.position", [1, 3], "3a Conflict"],
  ["MAIN.footerPair", ["~/ProtonDrive", "/Drive/RemoteFolder"], "2a Settled"],
  ["MAIN.footerTotals", [386000000, 1100000000], "2a Syncing"],
  ["MAIN.pausedSub", [7, "13:20"], "10a Paused"],
  ["MAIN.settledSub", ["2 minutes ago", 12480, 41200000000], "2a Settled"],
  ["MAIN.settledSubTime", ["2 minutes ago"], "2a Settled"],
  ["MAIN.syncingSub", ["14 seconds ago", 2, 1], "2a Syncing"],
  ["ONBOARDING.alreadyMatch", [11798], "9a Review"],
  ["ONBOARDING.conflictsKept", [2], "9a First sync"],
  ["ONBOARDING.differ", [2], "9a Review"],
  ["ONBOARDING.doneSub", [12480, 41_200_000_000], "9a Consent"],
  ["ONBOARDING.doneSubPhase1", [2], "9a Consent"],
  ["ONBOARDING.freeSpace", [38_400_000_000, 214_000_000_000], "9a Review"],
  ["ONBOARDING.freeSpaceHave", [214_000_000_000], "9a Review"],
  ["ONBOARDING.progressDone", [159, 471], "9a First sync"],
  ["ONBOARDING.progressSub", [159, 471, "about 17 minutes left"], "9a First sync"],
  ["ONBOARDING.receivedCount", [115], "9a First sync"],
  ["ONBOARDING.seeAllActions", [471], "9a Review"],
  ["ONBOARDING.sentCount", [44], "9a First sync"],
  ["ONBOARDING.sideUnit", [128, 1_400_000_000], "9a Review"],
  ["ONBOARDING.signedIn", ["you@proton.me", 39_100_000_000, 500_000_000_000], "9a Folders"],
  ["ONBOARDING.workedOut", ["40 seconds ago", "about 25 minutes"], "9a Review"],
  ["ONBOARDING.workedOutPlain", ["40 seconds ago"], "9a Review"],
  ["TRAY.retrying", ["40s", "13:58"], "10a Offline"],
  ["TRAY.unreachableBody", [4], "10a Offline"],
];

/** Every own-text string in every frame, and which frames said it. */
const saidBy = new Map();
for (const file of readdirSync(FRAMES)) {
  if (file === "index.json" || !file.endsWith(".json")) continue;
  const frame = JSON.parse(readFileSync(join(FRAMES, file), "utf8"));
  for (const node of frame.nodes) {
    // Own text, the joined subtree text (a sentence split by an inline <strong>), and every
    // user-visible attribute — a placeholder is copy just as much as a paragraph is.
    for (const said of [node.text, node.fullText, ...Object.values(node.attrs ?? {})]) {
      if (!said) continue;
      if (!saidBy.has(said)) saidBy.set(said, []);
      saidBy.get(said).push(frame.label);
    }
  }
}
// Also index the concatenated text of each frame, so a sentence split across inline children (a
// <strong> mid-paragraph — the deck has 25 of them) is still found.
const frameText = new Map();
for (const file of readdirSync(FRAMES)) {
  if (file === "index.json" || !file.endsWith(".json")) continue;
  const frame = JSON.parse(readFileSync(join(FRAMES, file), "utf8"));
  // Each piece is whitespace-normalised on its own and then joined with a separator that cannot
  // occur in copy, so a match can never span two unrelated nodes. (A control character here trips
  // eslint's no-control-regex; the pilcrow is printable, absent from the deck, and just as unique.)
  frameText.set(
    frame.label,
    frame.nodes
      .flatMap((n) => [n.fullText ?? n.text ?? "", ...Object.values(n.attrs ?? {})])
      .map((t) => t.replace(/\s+/g, " ").trim())
      .filter(Boolean)
      .join(" ¶ "),
  );
}

/**
 * Walk the exported constants, collecting `PATH -> string` for every fixed string AND the path of
 * every template.
 *
 * COLLECTING TEMPLATES IS #372, AND IT CLOSES A HOLE THE HEADER ABOVE OPENED. Only strings were
 * collected, so turning a checked constant into a template removed it from this gate silently, with
 * no failure anywhere: measured on the #193 branch, `340/340 matched` became `339/339 matched`, both
 * green, and the lower number was quoted as verification. Neither existing guard could see it —
 * `DRAWN` fails on a path that stops being a function, `NOT_DRAWN` fails on one that starts being
 * one, and a string that becomes a template is in neither table.
 */
const strings = [];
const templates = [];
const walk = (value, path) => {
  if (typeof value === "string") strings.push([path, value]);
  else if (typeof value === "function") templates.push(path);
  else if (value && typeof value === "object") {
    for (const [k, v] of Object.entries(value)) walk(v, `${path}.${k}`);
  }
};
for (const [group, value] of Object.entries(COPY)) walk(value, group);

/** Resolve a dotted path into the deck. Returns undefined rather than throwing on a bad segment. */
const at = (path) => path.split(".").reduce((node, key) => (node == null ? node : node[key]), COPY);

// Render each drawn template and require its result IN THE FRAME THE TABLE NAMES — not merely
// somewhere in scope, which is what the first version did by ignoring the third tuple element
// (Copilot caught it). The label is the table's whole claim: these arguments were read off THAT
// frame, and a check that accepts any frame lets the claim rot while staying green. Verified by
// pointing one entry at the wrong frame and watching the build fail.
//
// A path that no longer names a function, or a label that is not an in-scope frame, is a build
// failure and not a skip: the point is that this table cannot become a list of things nobody checks.
const templateErrors = [];
const drawnChecks = [];
for (const [path, args, label] of DRAWN) {
  const fn = at(path);
  const shown = `${path}(${args.map((a) => JSON.stringify(a)).join(", ")})`;
  if (typeof fn !== "function") {
    templateErrors.push(
      `${shown} is ${fn === undefined ? "not in the deck" : `a ${typeof fn}`}, not a template`,
    );
    continue;
  }
  if (!frameText.has(label)) {
    templateErrors.push(`${shown} names frame "${label}", which is not an in-scope frame`);
    continue;
  }
  drawnChecks.push([shown, fn(...args), label]);
}

// EVERY EXEMPTION MUST NAME A STRING THE GATE WOULD OTHERWISE CHECK, and the header explains why
// this is worth a check of its own: `NOT_DRAWN` is a `continue` before the lookup, so a wrong entry
// is never contradicted by anything. Two ways to be wrong, and both are silent — a path that no
// longer exists (the string was renamed or deleted), and a path that names a TEMPLATE, which `walk`
// never collects, so the exemption excuses nothing while still subtracting one from the total
// printed below. Nearly shipped the second one with `MAIN.failedSub` (#246).
const staleExemptions = [];
const collected = new Set(strings.map(([path]) => path));
for (const path of NOT_DRAWN.keys()) {
  if (collected.has(path)) continue;
  const value = at(path);
  staleExemptions.push(
    `${path} is ${
      value === undefined
        ? "not in the deck"
        : typeof value === "function"
          ? "a template — NOT_DRAWN exempts strings; a template belongs in DRAWN or UNGATED_TEMPLATES"
          : `a ${typeof value}, not a string`
    }`,
  );
}

/**
 * EVERY TEMPLATE IS IN EXACTLY ONE TABLE (#372).
 *
 * `DRAWN` says "a frame draws this, at these arguments". `UNGATED_TEMPLATES` says "no `DRAWN` row is
 * possible, and here is why". A template in neither is the hole this closes; a template in both is
 * two tables disagreeing about the same path, which is how a suppression outlives the thing it
 * suppresses.
 *
 * WHAT THIS LIST SELF-INVALIDATES ON. It fails when an entry stops being a template (renamed,
 * deleted, or turned back into a constant — which puts it under the string checks, where it belongs)
 * and when an entry is also in `DRAWN`.
 *
 * IT ALSO FAILS WHEN A FRAME STARTS DRAWING THE ENTRY, which the first version of this comment
 * declared impossible — "knowing that needs the arguments, and not having them is the entry's whole
 * reason for existing". That was false for most of them, and #372's acceptance criteria asked for
 * the check outright ("or is no longer undrawn, **fails the build**"). Most of these templates have
 * a fixed skeleton no frame contains at ANY argument — `The full sweep didn't start —`,
 * `created on this computer to hold them.` — so `absent` names one such fragment and the gate fails
 * if a frame ever contains it. 15 of the 21 carry one.
 *
 * THE MECHANISM CHECKS ITSELF, which is what keeps a bad fragment from being worse than none: "the
 * fragment is present" IS the failure condition, so a fragment that was never absent fails on the
 * run that introduces it rather than sitting there never firing.
 *
 * `absent: null` is REQUIRED rather than omitted, so a new entry has to answer the question. The six
 * that declare it are the ones whose fixed words the frames legitimately contain — `MAIN.batchFiles`
 * and `PLAN.checkingProgressBare` both render `N files`; `SETTINGS.monthEdgeNote` and
 * `DELETIONS.armedBodyFile` are drawn in full, at an argument the app cannot produce; `MAIN.andMore`
 * and `CONFLICTS.edited` have no fixed run longer than a word. For those six the reason carries the
 * weight alone, which is why each names a call site or a frame rather than asserting a category.
 */
const templateCoverage = [];
const drawnPaths = new Set(DRAWN.map(([path]) => path));
for (const path of templates) {
  const inDrawn = drawnPaths.has(path);
  const inUngated = UNGATED_TEMPLATES.has(path);
  if (inDrawn && inUngated) {
    templateCoverage.push(`${path} is in BOTH tables — DRAWN renders it, UNGATED_TEMPLATES excuses it`);
  } else if (!inDrawn && !inUngated) {
    templateCoverage.push(
      `${path} is a template in neither table — add a DRAWN row, or an UNGATED_TEMPLATES reason`,
    );
  }
}
// A DECLARED `absent` FRAGMENT THAT A FRAME NOW CONTAINS means the entry has gone stale: the screen
// this template belongs to got drawn, and the entry still says nobody draws it. This is the
// direction #372 asked for and the first version of this file declared impossible.
//
// It doubles as the fragment's own validation. "The fragment is present" IS the failure condition,
// so a fragment that was never absent fails on the run that introduces it — this list cannot
// accumulate probes that quietly never fire, which is the only way a check like this is worth less
// than no check at all.
for (const [path, entry] of UNGATED_TEMPLATES) {
  if (entry.absent == null) continue;
  const drawnIn = [...frameText].find(([, text]) => text.includes(entry.absent));
  if (drawnIn) {
    templateCoverage.push(
      `${path} is recorded as ungatable, but ${drawnIn[0]} now draws ` +
        `${JSON.stringify(entry.absent)} — give it a DRAWN row, or correct the reason`,
    );
  }
  // AND THE FRAGMENT MUST BE THE TEMPLATE'S OWN TEXT. Absence from the frames is only half of what
  // makes a probe work: one that the template can never render — a typo, a hyphen where the deck
  // has an em-dash, a sentence copied from the entry above — is absent for ever, passes the check
  // above, and NEVER FIRES. That is the one failure the "a present fragment fails immediately"
  // property does not cover, and it is the failure mode a probe list decays into.
  //
  // Rendering at a grid rather than at one argument, because these take counts, paths, objects and
  // optional flags; a fragment reachable at ANY of them is the template's own words.
  const fn = at(path);
  const reachable =
    typeof fn === "function" &&
    UNGATED_PROBE_ARGS.some((args) => {
      try {
        const rendered = fn(...args);
        return typeof rendered === "string" && rendered.includes(entry.absent);
      } catch {
        return false;
      }
    });
  if (!reachable) {
    templateCoverage.push(
      `${path}'s absent fragment ${JSON.stringify(entry.absent)} is not something the template ` +
        `renders at any probed argument — it could never fire, so it is not a staleness check`,
    );
  }
}

// The other direction: an entry excusing something that is no longer a template.
const templatePaths = new Set(templates);
for (const path of UNGATED_TEMPLATES.keys()) {
  if (templatePaths.has(path)) continue;
  const value = at(path);
  templateCoverage.push(
    `${path} is ${
      value === undefined
        ? "not in the deck"
        : typeof value === "string"
          ? "a string now, not a template — it belongs in the string checks, or in NOT_DRAWN"
          : `a ${typeof value}, not a template`
    }`,
  );
}

const missing = [];
const found = [];
for (const [path, text] of strings) {
  if (NOT_DRAWN.has(path)) continue;
  if (saidBy.has(text)) {
    found.push(path);
    continue;
  }
  const inFrame = [...frameText.entries()].find(([, all]) => all.includes(text));
  if (inFrame) {
    found.push(path);
    continue;
  }
  missing.push([path, text]);
}

// The templates, each against ITS OWN frame rather than against all of them.
for (const [shown, text, label] of drawnChecks) {
  if (frameText.get(label).includes(text)) found.push(shown);
  else missing.push([`${shown} — expected in ${label}`, text]);
}

console.log(
  `fidelity:copy — ${found.length}/${strings.length - NOT_DRAWN.size + drawnChecks.length} drawn strings matched ` +
    `(${drawnChecks.length} of them templates rendered at the arguments their own frame draws), ` +
    `${NOT_DRAWN.size} exempt (no frame draws them), ${missing.length} missing`,
);
// PRINTED EVERY RUN, because #372's defect was a total that dropped by one in silence. The template
// figures are the denominator that makes the drawn count mean something: "72 of 120" says what
// "340/340 matched" cannot.
//
// COUNTED IN PATHS, and the line above counts ROWS — they differ by design, so both are named. A
// path may be drawn on two frames and earns a `DRAWN` row for each (`CONFLICTS.versionDiff` and
// `CONFLICTS.happened` are, which is why 74 rows cover 72 paths). Printing 74 here would say 74 of
// the deck's 120 templates are covered, and 72 are.
console.log(
  `fidelity:copy — ${templates.length} templates: ${drawnPaths.size} covered by a DRAWN row ` +
    `(${DRAWN.length} rows), ${UNGATED_TEMPLATES.size} recorded as ungatable, ` +
    `${templateCoverage.length} unaccounted for`,
);

if (templateErrors.length) {
  console.error("\nEntries in the drawn-template table that are no longer templates:\n");
  for (const problem of templateErrors) console.error(`  ${problem}`);
}

if (staleExemptions.length) {
  console.error("\nExemptions in NOT_DRAWN that excuse nothing:\n");
  for (const problem of staleExemptions) console.error(`  ${problem}`);
  console.error(
    "\nAn exemption is a claim that a string exists and no frame draws it. Delete the entry, or fix the path.",
  );
}

if (missing.length) {
  console.error("\nStrings in ui/copy.js that no in-scope frame contains:\n");
  for (const [path, text] of missing) {
    console.error(`  ${path}`);
    console.error(`    "${text}"`);
    // The most likely cause by far, so say it rather than making everyone rediscover it.
    const straightened = text.replace(/[’‘]/g, "'").replace(/[“”]/g, '"').replace(/—/g, "-");
    if (
      straightened !== text &&
      (saidBy.has(straightened) || [...frameText.values()].some((t) => t.includes(straightened)))
    ) {
      console.error("    ^ the frame has this with STRAIGHT quotes/dashes — the deck's are typographic");
    }
  }
  console.error(`\nfidelity:copy: ${missing.length} string(s) do not match the frames.`);
}

if (templateCoverage.length) {
  console.error("\nTemplates in neither table, or excused by an entry that no longer fits:\n");
  for (const problem of templateCoverage) console.error(`  ${problem}`);
  console.error(
    "\nA template checked by nothing is how a string leaves this gate silently (#372). Give it a" +
      " DRAWN row if a frame draws it, or an UNGATED_TEMPLATES reason if none can.",
  );
}

if (missing.length || templateErrors.length || staleExemptions.length || templateCoverage.length)
  process.exit(1);
