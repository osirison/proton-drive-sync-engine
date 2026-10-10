// The wiring gate for "a write acts on the pair it was drawn for" (#102 phase 5a-2, acceptance 5 and 6).
//
// `api.test.js` proves the facade refuses a write with no pair, `pair-ledger.test.js` proves the
// state that describes one pair is dropped on a switch, and `selection_tests.rs` proves Rust's side.
// None of them runs `app.js`, and `app.js` is where the pair is CAPTURED: which row a button belongs
// to, which pair a plan was made for, what a press commits to. It cannot be imported (it boots on
// load), so this drives the real page against a scripted stand-in for Tauri's bridge — a Node object
// that answers each command and **holds a reply open when told to**, which is what makes the window
// between "the person pressed it" and "the daemon answered" a thing a test can stand in.
//
// A hundred and five scenarios (the first sixteen are phases 5a-2, 5d and 5b-1's; forty-three are phase 5c-1's —
// twenty-one built with the selector, the nineteen its review added and the three its second review added — and the last fifteen
// are phase 5e's, the notifications: the six it was built with and the nine its review added; both below the list —
// the fifteen after them are phase 5c-2's, adding and removing folders, below the notifications', the ten after those
// are its review's (84 to 93), and the last two are that review's final round (94 and 95), all at the end of the
// file). The first four are each a way a write can land on a different pair than the one it was
// drawn for — each was a bug before the capture existed or would be again if it were removed. The fifth
// and sixth are the first-run rule at two pairs and at one. The next three are the rest of the capture:
// a late READ, the decision on a conflict, and the tray panel's pin. The next two are the tray panel's
// rows at two folders (#102 phase 5d) and its `Review them`, which names the folder it is drawn for. The
// last four are the Settings screen's (#102 phase 5b-1): what is staged belongs to one folder.
//
//   1. THE TAIL OF A DECISION. `Move to Proton's Trash` approves, waits for the daemon, then nudges a
//      sync. The selection moves while the approval is in flight; the nudge must still go where the
//      approval went.
//   2. THE PRESS OF `Run this sync`. It sends the plan's deletion approvals one at a time and then
//      applies by token. A switch part-way resets the plan screen; the apply must still be the plan
//      that was reviewed, for the pair it was reviewed for.
//   3. A SWITCH IS A CHANGE OF SHAPE. Two folders withhold the same path at the same fingerprint, so
//      the two queues draw identical cards; if the screen patched instead of rebuilding, the new
//      folder's cards would carry the old folder's handlers. `Keep` on the card now showing must go
//      to the folder now showing.
//   4. THE HERO'S BUTTONS. Two folders in the same state draw the same hero, so a switch patches the
//      screen instead of rebuilding it, and its buttons are the ones built for the previous folder.
//      `Pause` must still pause the folder the hero is now about.
//   5. A PAIR THAT HAS NEVER SYNCED. At two folders the first-run wizard (the first-folder flow)
//      must stay shut, and the window must say the pair has
//      not synced rather than `Everything is up to date`; ...
//   6. ... and at one folder the same reply still opens the wizard, because it is the count and not
//      the state that decides.
//   7. A LATE READ. Two folders hold a conflict at the SAME relative path, so the reads for them differ
//      only in the folder they were issued for. The older read answers after the selection has moved
//      and the newer one has been issued; its content must not become the card now showing, on the
//      screen where the person chooses which version to destroy.
//   8. THE DECISION ON A CONFLICT. A card drawn for one folder is pressed after the selection has
//      moved to another; the decision goes to the folder the card was drawn for.
//   8b. ... AND WHAT FOLLOWS IT. The decision lands after the selection has moved: its continuation
//      (the tally, the position, the cached bytes) belongs to the screen that was shown, not the one
//      that is, and must not touch the folder now on screen.
//   9. THE TRAY PANEL'S PIN. The panel shows the default pair whatever the window has selected, from its
//      very FIRST poll: it asks for it with a command of its own (`tray_status`) that has no pair to
//      name, so there is no first call that can mean the window's selection (phase 5a-2 left that as a
//      recorded gap: before the roster was heard the poll named no pair, which Rust reads as the
//      selected one).
//  10. THE TRAY PANEL'S ROWS AT TWO FOLDERS. The panel is the worst folder's, named, with one pause row
//      per folder; pressing a folder's row sends THAT folder's id, and the panel is repainted from the
//      status that comes back, not from the folder the row was for.
//  11. THE TRAY PANEL'S `Review them`. A deletion waiting in the folder the panel is NOT about still
//      shows (the count is every folder's), and the button sends the id of the folder that holds it —
//      and when the decisions move to the other folder while the panel stays a needs-you panel, it is
//      patched in place, so the button it keeps must send the NEW folder's id, not the one it was built
//      for.
//  12. THE SAVE OF STAGED SETTINGS. A policy is staged against one folder and the selection moves before
//      `Save` is pressed on the bar that was drawn for it: the write names the folder the edit was typed
//      for, with that folder's update, and nothing is written for the folder now showing.
//  13. WHAT IS STAGED BELONGS TO ONE FOLDER. The folder switched to draws its own saved settings and
//      none of the other's edits (and has nothing to save), and switching back finds the edit still there.
//  14. A READ NAMES THE FOLDER ON SCREEN. Once a status has said which folder is shown, the settings
//      read asks about THAT folder; and a reply is filed under the folder it says it describes.
//  15. WHAT THE WINDOW READS BACK IS THE FOLDER ON SCREEN'S. The Details dialog states the scan interval
//      of the folder it is about, from the config reply filed for that folder.
//
// PHASE 5c-1 (the selector and the scoped window; the carried reviews of #439, #441/#443 and #445):
//
//  16. THE PILL IS DRAWN AT TWO FOLDERS AND AT NO FEWER, and the header grows and loses it across a poll
//      (`updateHeader`'s shape check); the hero's button names its folder exactly while the pill is there.
//  17. THE KEYBOARD. Down on the pill opens the list on the chosen folder, the arrows/Home/End walk it, Esc
//      closes it and returns to the pill, Enter chooses (Rust is told), a press elsewhere closes it — and
//      three polls leave the pill, the list and the row the keyboard is on the SAME nodes.
//  18. A LATE STATUS. A poll that left before the person chose another folder answers after; it must not take
//      the window back to the folder that was left.
//  19. A SWITCH DISARMS. A typed-`DELETE` gate armed for one folder is not armed on the other folder's
//      identical row.
//  20. THE FOOTER LINE names the folder on screen.
//  21. THE RING is on the pill when another folder waits on a person (a deletion, or a conflict the poll's
//      60-second scan found), the chip stays the selected folder's, a number is never on the pill.
//  22. `pause_unsaved`: said under the hero from the hero's own press, from the tray row's event, only for the
//      folder on screen, retired by the next press.
//  23. A FOLDER THE DAEMON DOES NOT RUN gets the notice and the one button, only when the settings file lists
//      it, and a restart that did not work quotes the daemon.
//  24-29. THE SETTINGS HANDLERS ARE THE SCREEN'S. Each of `onRoot`, `onField`, `onEvents`, `onRemoveRule`,
//      `onAddRule` and `chooseLocalRoot` is used after the selection has moved (a click landing in the gap);
//      back on the folder it was drawn for, `Save` writes what was staged, to that folder and no other.
//  30. `onRestart` retires the notice of the folder whose bar it was drawn on.
//  31. `loaded`: a folder whose settings have not arrived draws none (no policy card chosen from another
//      folder's file or an empty one).
//  32. `Discard changes` discards the folder on screen's edits and no other folder's.
//  33. A KEYSTROKE IN A SAVE IN FLIGHT is still staged when the save lands.
//  34-35. A FOLDER NAMED `constructor` / `__proto__` is chosen, shown, staged for and saved like any other.
//  36. THE TALLY of a late decision counts the visit it was made on (the cleared screen says `one`, not two).
//
// THE REVIEW OF #447 (nineteen more):
//
//  37. THE MARKER'S SECOND FORM. Another folder that failed, or is unavailable (an unplugged drive), marks the
//      pill with a SOLID dot where the decision ring is hollow; a paused folder marks nothing; the folder on
//      screen failing is the chip's and the hero's, never the pill's.
//  38. WHEN BOTH APPLY the problem form is the one drawn, as one marker, and the pill is the same node when
//      its marker changes form.
//  39. A STOPPED DAEMON. Every status read fails: no row of the list says `up to date`, none counts, and the
//      pill carries no marker from the last answer; the live list is back when the daemon is.
//  40-41. AN UNSAVED PAUSE ENDS WHEN IT STOPS BEING TRUE: a resume done elsewhere (no event reaches the
//      window) retires `Paused, but not saved`, and a pause done elsewhere retires `Resumed, but not saved`;
//      and a status reply that LEFT BEFORE the notice cannot end it.
//  42. ...IS KEPT PER FOLDER: the tray's event for a folder that is not on screen is there when that folder is
//      looked at, and one for a folder the daemon does not list is not kept.
//  43. ...AND IS NOT KEPT FOR A DAEMON THAT STOPPED ANSWERING.
//  44. A FAILED RESTART belongs to the folder the notice was for; the next folder's notice starts over.
//  45-50. THE SETTINGS HANDLERS THE FIRST PASS LEFT UNPINNED: `onPolicy`, `onDisposal`, `onSchedule`,
//      `onDraft`, `onAddInclude` and `onSweep` each act on the folder they were drawn for.
//  51. A SWITCH DROPS THE PLAN made for the folder that was left, on the real page and not only in the
//      ledger's source scan.
//  52. ONE TAB STOP in the list: the chosen folder's row, and it moves with the choice.
//  53. THE KEYBOARD LEAVING closes the list; moving inside it, or back to the pill, does not.
//  54. A LONG LIST (25 folders) scrolls inside the window by wheel and by arrows, and the focused row is
//      always visible in it.
//  55. A SCAN THAT NEVER RETURNS for a folder that is not on screen does not stop the shown folder's polls,
//      and is one scan, not one per poll.
//
// THE SECOND REVIEW OF #447 (three more):
//
//  56. THE TRAY PANEL OVER A STOPPED DAEMON at two folders is the one-folder stopped-daemon panel: no `Up to
//      date`, no `Sync now`, no pause row per folder (the store keeps the last folder list across a failed read).
//  57. A FAILED READ OF A FOLDER THAT IS NOT ON SCREEN is filed under that folder: the chip, the list's rows and
//      the pill's ring of the folder that IS on screen do not change.
//  58. THE LIST SCROLLS FROM THE TWELFTH FOLDER: 31px rows, 11 of them 355px, under the 360px cap.
//
// PHASE 5e (notifications per folder; the bridge answers `send_notification` with nothing and records the payload, so a
// "banner" here is a recorded call and no desktop is touched):
//
//  59. A BANNER ABOUT ANOTHER FOLDER names it (`Drive Sync · photos`, and `pair` in the payload), and `Keep them` on it
//      keeps THAT folder's permanent deletions and no other's — the window shows a folder with a permanent deletion of
//      its own, which the press must not touch.
//  60. `Review` ON ANOTHER FOLDER'S BANNER selects the folder BEFORE it opens the Deletions screen: with the selection
//      held in flight the window has not moved, and once it lands the screen is that folder's queue and not the one that
//      was on screen.
//  61. `Try again now` ON ANOTHER FOLDER'S BANNER is a `syncnow` for that folder and not the tray's sync-every-unpaused row.
//  62. A DAEMON THAT STOPS ANSWERING raises no banner from the roster the store still holds: a folder five minutes short of
//      "nothing has synced for a day" crosses it on a clock moved ten minutes while the daemon is down, and says nothing;
//      the same roster with the daemon back says it, which is what makes the first half evidence.
//  63. ANOTHER FOLDER'S QUEUE IS FETCHED only while its summary counts one.
//  64. AT ONE FOLDER the banner has no `pair` and the line is `Drive Sync`, `Keep them` keeps the one folder's items and
//      `Try again now` is the tray's row, exactly as before.
//
// PHASE 5e, REVIEW ROUND ("only what is known is said": a folder that is not on screen is summarised on every poll, but
// the list behind its count and its conflict scan land later):
//
//  65. A BANNER NEVER NAMES A FILE `Keep them` ALREADY KEPT. The press drains the folder's queue and nothing refetches at
//      0; a new deletion's FIRST poll runs the notifier before its read has come home, and must not build from the old
//      list. The banner that follows names the new file alone, and its press keeps that file alone.
//  66. WHILE ANOTHER FOLDER'S LIST IS OLDER THAN ITS COUNT (its read held in flight) the folder list counts the summary's
//      number and not the old list's length, and `Keep them` keeps nothing; once the read lands, the same press keeps it.
//  67. A STANDING BANNER IS NOT SAID AGAIN BY THE NEXT LAUNCH — five scenarios, one per place it can stand: a deletion
//      and a conflict in a folder that is not on screen (the two that were said again, because the first poll of a
//      launch had the count but not the list, and had not scanned), and as controls the same in the folder on screen
//      and a deletion at one folder.
//  68. A RELAUNCH DOES NOT GO QUIET about such a folder: a different queue after it is said.
//  69. WHEN THE DEFAULT FOLDER IS REMOVED the next folder's own conflict at the same relative path is still said (a
//      conflict's signature is only its paths, so the removed folder's bare key silenced it).
//
// THE FOLDER THAT HAS NOT HAD ITS TURN (the live report and #455; four more, and the fifth and ninth above were
// rewritten with it). The daemon runs one pass at a time, so beside other folders a folder with no finished pass is
// `queued`: it says it is waiting, and for which folder, or that it is starting — never `up to date`, and never the
// tray's `Open Drive Sync to choose your two folders` in the window. These four replay the payloads the real Rust
// sends (`gui/test/never-synced-payloads.json`, held by `selection_tests.rs`), not a stand-in's guess at them:
//
//  96. WAITING BEHIND A RUNNING PASS: `documents` is mid-pass, `photos` is on screen. The hero, its sub-line, the
//      chip and the buttons say photos waits for documents; the pill's list and the Settings list say
//      `syncing`, `waiting for documents`, `waiting for documents` — no folder reads `up to date`.
//  97. THE FOLDER THAT READ `up to date`: selected, `videos` is drawn as waiting too, and its buttons name it.
//  98. STARTING: with nothing popped yet every folder is `starting`, in the window, the list and the tray panel, and
//      the panel has no button and a pause row for each folder.
//  99. A FOLDER THE FILE LISTS BESIDE A NEVER-SYNCED ONE (the daemon runs one, the app knows of two): the window's
//      sub-line is `It starts on its own.` and not the tray's sentence (#455).
//
// WHAT IT CANNOT SEE: it scripts the bridge, so it proves the facade and the screens agree with each
// other, not that the real Rust agrees with either (that is `selection_tests.rs`); and it drives the
// window, except the tray panel scenarios above, which drive `?surface=tray` through the same bridge.

import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import puppeteer from "puppeteer";
import { serve } from "./serve.mjs";
import {
  CHROME,
  CONFLICTS,
  DELETIONS,
  FOLDERS,
  MAIN,
  ONBOARDING,
  PLAN,
  SETTINGS,
  TRAY,
} from "../../src/js/ui/copy.js";
import { EMPTY_CONFIG } from "../../src/js/api.js";

// The payloads the real Rust sends for three folders none of which has finished a pass, at two moments
// (`running`: the default folder is mid-pass; `starting`: nothing has been popped yet), one per folder it
// describes. Written by `selection_tests.rs` (`never_synced_payloads_are_what_the_page_scenarios_replay`), which
// fails when this build sends anything else — so a scenario that replays them is shown what the daemon says.
const NEVER_SYNCED = JSON.parse(
  readFileSync(fileURLToPath(new URL("../../test/never-synced-payloads.json", import.meta.url)), "utf8"),
);

// The default roster of every Bridge that does not name its own. FROZEN: it is one array shared by all of them,
// and a scenario that pushed a folder onto it (`bridge.names.push(...)`) silently gave every later scenario a
// third folder with no queue — the status handler threw and the rest of the run failed far from the cause.
// Replace it (`bridge.names = [...bridge.names, "x"]`), never mutate it.
const PAIRS = Object.freeze(["docs", "photos"]);

const summaryOf = (name, pending) => ({
  name,
  local_root: `/home/u/${name}`,
  remote_root: `/Drive/${name}`,
  db_path: `/home/u/${name}/.sync/sync_index.db`,
  paused: false,
  syncing: false,
  reconcile_seq: 1,
  last_sync_epoch_secs: 1_750_000_000,
  last_error: null,
  pending_changes: 0,
  pending_deletions: pending,
});

const deletion = (path) => ({
  path,
  direction: "remote",
  entity_kind: "file",
  fingerprint: `fp-${path}`,
  detected_epoch_secs: 1_750_000_000,
  first_seen_epoch_secs: 1_750_000_000,
});

const delay = (ms) =>
  new Promise((resolve) => {
    setTimeout(resolve, ms);
  });

/** Poll `check` until it is truthy; reject with `what` if it never is. */
async function until(what, check, ms = 6000) {
  const stop = Date.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > stop) throw new Error(`timed out waiting for ${what}`);
    await delay(25);
  }
}

/** One conflict, found at the same relative path in both folders: the reads for them differ in the folder alone. */
const CONFLICT = { original: "note.txt", sidecar: "note.proton-cloud.txt", kind: "content" };

/** A scripted daemon: answers every command the window sends, records them, and holds replies on request. */
class Bridge {
  constructor(
    queues,
    {
      names = PAIRS,
      neverSynced = [],
      conflicts = {},
      selected = "docs",
      states = {},
      configs = {},
      pairUnknown = null,
      roster = null,
      trackPause = false,
      notifyPolicy = "never",
      lastSync = 1_750_000_000,
      replay = null,
    } = {},
  ) {
    this.names = names;
    // The status payloads THIS BUILD'S RUST sends, by the folder each describes (`never-synced-payloads.json`, held
    // by `selection_tests.rs`). When set, a status read answers with those bytes instead of what this stand-in
    // would compose: the page is then shown what the daemon says, not a guess at it.
    this.replay = replay;
    // Each folder's pass counter (`reconcile_seq`), 1 unless a scenario moves it: `finishPass` is how a
    // scenario says a folder's pass has completed, and the merge dialog of an added folder waits for exactly that.
    this.seqs = {};
    // Folder adding and removing (#102 phase 5c-2): what the engine's check says about a name and a folder,
    // what the measurement of a side answers (or refuses with), an index a folder would resume, and the
    // sentence the add command refuses with. The roster is the settings FILE's; `restart_service` is what makes
    // the daemon run it.
    this.survivors = {};
    this.nameRefusals = {};
    this.probeFailures = {};
    this.addRefusal = null;
    // A daemon that restarts and still does not run the new folder: the file lists it and the daemon does not.
    this.refuseToList = false;
    this.setAsideDir = "/home/u/.local/state/proton-sync/removed-pairs";
    this.removeReply = null;
    // The sentence a `write_config` is refused with, or null for a save that lands.
    this.writeRefusal = null;
    // The accounts of earlier removals an `add_pair` finished (`settled_earlier[].message`), as it replies them.
    this.addSettled = [];
    // A stand-in that answers as Rust does once a folder has left the file: a folder asked for by name that the
    // roster does not have is REFUSED (`read_config`), and a selection that names none falls back to the first
    // folder. Off, the stand-in is kind to a selection that outlived its folder, which hides the bug it is for.
    this.strict = false;
    // Folders `select_pair` refuses although the roster lists them (strict only): the daemon stopped running
    // one since the window last heard its list. Refused as Rust refuses, with the selection left where it was.
    this.selectRefused = new Set();
    // The `notify_policy` the window reads at boot. `never` for every scenario that is not about banners
    // (the default stays what it was), so only the ones that ask for it can raise one.
    this.notifyPolicy = notifyPolicy;
    // When every folder last synced, as a plain number (the same fixed instant as ever unless a scenario
    // asks). The notifier measures "nothing has synced for a day" against the page's own clock, so a
    // scenario about banners gives a recent one and only the folder it names is old.
    this.lastSync = lastSync;
    // folder -> its own last sync, over the default above.
    this.lastSyncOf = {};
    // The control socket does not answer: a status read comes home as Rust's unreachable payload.
    this.down = false;
    // Folders whose NAMED status read fails while the daemon is otherwise fine (a socket error on one
    // request): the reply is the same unreachable payload, stamped with the folder on screen.
    this.failFor = new Set();
    // Whether `pause`/`resume` change what the next status says. Off by default — a scenario that presses
    // Pause twice relies on the hero offering it twice — and on for the ones about WHEN a notice ends.
    this.trackPause = trackPause;
    this.pausedPairs = new Set();
    // The folder the window remembered and the daemon does not run: what a selection read that fell
    // back to the default folder says (`pair_unknown`), until a restart.
    this.pairUnknown = pairUnknown;
    // The folders the settings FILE lists (`read_config.pairs`), which can be more than the daemon runs.
    this.roster = roster;
    // The reason a `pause`/`resume` answers `pause_unsaved` with, or null for a saved one.
    this.unsaved = null;
    // How `restart_service` ends, and what `resync` and `choose_folder` answer.
    this.restartEnding = "restarted";
    this.resyncError = null;
    this.picked = "/picked/folder";
    // Replies composed NOW and delivered after `release()`: the answer to a request that left before
    // something happened. `holds` is the other kind, where the daemon answers only once released.
    this.lateHolds = new Map();
    // pair -> the per-pair values its `read_config` reply carries beyond the empty config's.
    this.configs = configs;
    this.neverSynced = neverSynced;
    // pair -> `{ state, rank, summary }`: what Rust derived for a folder that is not simply idle.
    this.states = states;
    this.conflicts = conflicts; // pair -> the conflicts a scan finds there
    this.calls = [];
    this.selected = selected;
    this.queues = queues; // pair -> pending deletions
    this.holds = new Map(); // command (or `command:pair`) -> { release }
    this.planFor = (pair) => ({
      report: {
        summary: { total: 1, remote_deletes: 1, destructive_actions: 1 },
        plan: [{ action: "remote_delete", path: `${pair}/gone.txt`, entity_kind: "file" }],
        cannot_sync: [],
      },
      requires_delete_gate: true,
      files_at_risk: [`${pair}/gone.txt`],
      token: `token-for-${pair}`,
      local_disposal: "recoverable",
    });
  }

  isPaused(pair) {
    return this.trackPause && this.pausedPairs.has(pair);
  }

  /** A folder's pass has completed: its counter moves, and it is no longer a folder that has never synced. */
  finishPass(pair) {
    this.seqs[pair] = (this.seqs[pair] ?? 1) + 1;
    this.neverSynced = this.neverSynced.filter((name) => name !== pair);
  }

  /**
   * The engine's answer to `check_add_pair`, as far as a scripted stand-in can be it. The names it refuses and
   * the index it finds are the scenario's to say (`nameRefusals`, `survivors`); the suggestion is the
   * folder's own name, lower-cased, which is what the command makes of `~/Photos`.
   */
  checkAdd(args) {
    const name = String(args?.name ?? "");
    const local = String(args?.init?.local_root ?? "");
    const remote = String(args?.init?.remote_root ?? "");
    const last = local.split("/").filter(Boolean).pop() ?? "";
    const suggested = last.toLowerCase().replace(/[^a-z0-9._]+/g, "-") || "folder";
    const known = this.roster ?? this.names;
    const taken = known.some((existing) => existing.toLowerCase() === name.toLowerCase());
    const nameError =
      this.nameRefusals[name] ??
      (name === ""
        ? "a `[[pair]]` table has an empty `name`: every pair needs a name"
        : taken
          ? `two \`[[pair]]\` tables are named \`${name}\``
          : null);
    const refusal =
      !nameError && local && remote && !local.startsWith("/") && !local.startsWith("~")
        ? `the folder \`${local}\` is not a full path`
        : null;
    return {
      suggested_name: suggested,
      name_error: nameError,
      refusal: nameError || !local || !remote ? null : refusal,
      surviving_index: nameError || refusal ? null : (this.survivors[local] ?? null),
      warnings: [],
    };
  }

  /** What Rust answers when the socket does not (`status_payload`): no reply, no roster, the app's own selection. */
  unreachable() {
    return { state: "unreachable", error: "connect: no such file or directory", selected: this.selected };
  }

  /**
   * The reply to a status read. `name` is the pair it DESCRIBES — the one the request named, else the
   * selected one, as Rust answers — and `selected` is the app's choice, which is not the same thing.
   */
  status(asked = this.selected) {
    const name = this.strict && !this.names.includes(asked) ? this.names[0] : asked;
    if (this.replay) return { ...(this.replay[name] ?? this.replay[this.names[0]]), selected: this.selected };
    const never = (pair) => this.neverSynced.includes(pair);
    // As `gui_core::state::derive` decides it. Beside other folders a folder with no finished pass is `queued`
    // (waiting for the folder that is running a pass, when one is) and never the wizard's `firstRun`; alone it
    // is `firstRun`. Paused outranks both, and a state a scenario gave the folder (failed, ...) outranks the wait.
    const crowded = this.names.length >= 2;
    const running = this.names.find((pair) => this.states[pair]?.summary?.syncing);
    const stateOf = (pair) =>
      this.isPaused(pair)
        ? "paused"
        : (this.states[pair]?.state ?? (never(pair) ? (crowded ? "queued" : "firstRun") : "idle"));
    const waitingOf = (pair) =>
      stateOf(pair) === "queued" && running && running !== pair ? { waiting_for: running } : {};
    const pairs = this.names.map((pair) => ({
      ...summaryOf(pair, this.queues[pair].length),
      reconcile_seq: this.seqs[pair] ?? 1,
      last_sync_epoch_secs: this.lastSyncOf[pair] ?? this.lastSync,
      ...(never(pair) ? { last_sync_epoch_secs: null } : {}),
      ...(this.states[pair]?.summary ?? {}),
      ...(this.isPaused(pair) ? { paused: true } : {}),
    }));
    return {
      // What `derive_state` says: a reachable daemon that has never synced THIS pair — or the state the
      // scenario gave it (`states`), which is how a folder is failed, paused or unavailable on screen.
      state: stateOf(name),
      ...waitingOf(name),
      // As Rust answers it: a selection that names a folder the daemon no longer runs reads as the default one.
      selected: this.strict && !this.names.includes(this.selected) ? this.names[0] : this.selected,
      ...(this.pairUnknown ? { pair_unknown: this.pairUnknown } : {}),
      pairs,
      pair_states: this.names.map((pair) => ({
        name: pair,
        state: stateOf(pair),
        rank: this.isPaused(pair) ? 1 : (this.states[pair]?.rank ?? (stateOf(pair) === "queued" ? 2 : 0)),
        ...waitingOf(pair),
      })),
      response: {
        status: "running",
        paused: this.isPaused(name),
        syncing: false,
        reconcile_seq: this.seqs[name] ?? 1,
        pending_changes: 0,
        message: "",
        last_sync_epoch_secs: never(name) ? null : (this.lastSyncOf[name] ?? this.lastSync),
        last_error: this.states[name]?.summary?.last_error ?? null,
        last_plan_summary: null,
        last_successful_sync_summary: null,
        status_history: [],
        pending_deletions: this.queues[name],
        failed_items: [],
        failed_item_count: 0,
        config: { local_root: `/home/u/${name}`, remote_root: `/Drive/${name}`, db_path: "/x/i.db" },
        activity: null,
        unsyncable: [],
        auth: "signed-in",
        pair: name,
        pairs,
      },
    };
  }

  /**
   * Hold the next reply to `command` until `release()` is called — to `command` sent for `pair` when
   * one is given, so two reads of the same command for two folders can be released in either order.
   */
  hold(command, pair) {
    let release;
    const released = new Promise((resolve) => {
      release = resolve;
    });
    this.holds.set(pair ? `${command}:${pair}` : command, { released, release });
    return release;
  }

  /**
   * Like `hold`, but the reply is COMPOSED when the request arrives and delivered after `release()`. The
   * answer to a request that left before the selection moved says what was true when it left — which is
   * what an in-flight reply for the folder that was just left is, and what an early hold cannot be.
   */
  holdLate(command, pair) {
    let release;
    const released = new Promise((resolve) => {
      release = resolve;
    });
    this.lateHolds.set(pair ? `${command}:${pair}` : command, { released, release });
    return release;
  }

  called(command) {
    return this.calls.filter((call) => call.cmd === command);
  }

  async handle(cmd, args) {
    this.calls.push({ cmd, args, selectedThen: this.selected });
    for (const key of [`${cmd}:${args?.pair}`, cmd]) {
      const late = this.lateHolds.get(key);
      if (late) {
        this.lateHolds.delete(key);
        const composed = this.answer(cmd, args);
        await late.released;
        return composed;
      }
    }
    for (const key of [`${cmd}:${args?.pair}`, cmd]) {
      const held = this.holds.get(key);
      if (held) {
        this.holds.delete(key);
        await held.released;
        break;
      }
    }
    return this.answer(cmd, args);
  }

  /** What the daemon says to one command, as of now. */
  answer(cmd, args) {
    switch (cmd) {
      case "fence":
        return null;
      case "get_status":
        return this.down || this.failFor.has(args?.pair)
          ? this.unreachable()
          : this.status(args?.pair ?? this.selected);
      case "tray_status":
        // What Rust answers: the DEFAULT pair, whatever the window has selected and whatever is asked.
        return this.down ? this.unreachable() : this.status(this.names[0]);
      case "tray_action":
        // A row of the panel: the reply is the panel's own status, never the addressed folder's.
        return this.down ? this.unreachable() : this.status(this.names[0]);
      case "read_config": {
        // Answered for the pair the request names, else the selected one — and says which, as Rust does.
        const known = this.roster ?? this.names;
        if (this.strict && args?.pair && !known.includes(args.pair)) {
          throw new Error(`the config has no folder pair named "${args.pair}"`);
        }
        const wanted = args?.pair ?? this.selected;
        const pair = this.strict && !known.includes(wanted) ? known[0] : wanted;
        return {
          ...EMPTY_CONFIG,
          exists: true,
          set_aside_dir: this.setAsideDir,
          pair,
          pairs: (this.roster ?? this.names).map((name) => ({
            name,
            local_root: `/home/u/${name}`,
            remote_root: `/Drive/${name}`,
          })),
          ...this.configs[pair],
        };
      }
      case "read_notify_policy":
        return this.notifyPolicy;
      case "check_add_pair":
        return this.checkAdd(args);
      case "probe_folder":
        if (this.probeFailures[args?.side]) throw new Error(this.probeFailures[args.side]);
        return {
          files: args?.side === "local" ? 1204 : 1190,
          bytes: args?.side === "local" ? 3_400_000_000 : null,
          truncated: false,
          unreadable_directories: 0,
        };
      case "add_pair": {
        if (this.addRefusal) throw new Error(this.addRefusal);
        const known = this.roster ?? this.names;
        this.roster = [...known, args.pair];
        this.queues[args.pair] = [];
        return {
          pair: args.pair,
          path: "/home/u/.config/proton-sync/proton-sync.toml",
          restart_needed: true,
          surviving_index: this.survivors[args.init?.local_root] ?? null,
          settled_earlier: this.addSettled.map((message) => ({ outcome: "moved", pair: "old", message })),
          warnings: [],
        };
      }
      case "remove_pair": {
        const known = this.roster ?? this.names;
        const first = known[0] === args.pair;
        this.roster = known.filter((name) => name !== args.pair);
        this.names = this.names.filter((name) => name !== args.pair);
        // Rust does not move the selection when its folder goes: the window has to (`strict` is Rust).
        if (!this.strict && this.selected === args.pair) this.selected = this.names[0];
        return (
          this.removeReply ?? {
            pair: args.pair,
            path: "/home/u/.config/proton-sync/proton-sync.toml",
            new_default: first ? (this.roster[0] ?? null) : null,
            restart: { ending: "restarted", detail: "restarted" },
            restart_needed: false,
            set_aside: {
              outcome: "moved",
              pair: args.pair,
              to: `${this.setAsideDir}/${args.pair}-1`,
              items: [],
              left_behind: [],
              notes: [],
              message: `The sync history of '${args.pair}' was moved to ${this.setAsideDir}/${args.pair}-1, outside every sync folder. Your files were not touched.`,
            },
            settled_earlier: [],
          }
        );
      }
      case "write_config":
        if (this.writeRefusal) throw new Error(this.writeRefusal);
        return null;
      case "check_cli":
        return { installed: true, distro: null };
      case "scan_conflicts":
        return this.conflicts[args?.pair ?? this.selected] ?? [];
      case "read_conflict_pair": {
        // Content that names its folder, so a card that shows the wrong folder's file says so in text.
        const side = (kind) => ({
          exists: true,
          size: 40,
          mtime_epoch_secs: 1_750_000_000,
          text: `MARK-${args?.pair}-${kind}\nline two`,
          binary_or_large: false,
        });
        return { original: side("mine"), sidecar: side("theirs"), happened: null };
      }
      case "resolve_conflict":
        // Settling a conflict removes it: the next scan of that folder finds one fewer.
        this.conflicts[args?.pair] = (this.conflicts[args?.pair] ?? []).filter(
          (conflict) => conflict.original !== args?.conflict?.original,
        );
        return null;
      case "select_pair":
        // As Rust: a folder the daemon does not run is refused, and the selection stays where it was.
        if (this.strict && (!this.names.includes(args?.name) || this.selectRefused.has(args?.name))) {
          throw new Error(`no folder pair named "${args?.name}"`);
        }
        this.selected = args?.name;
        return args?.name;
      case "pause":
      case "resume": {
        // The daemon applies it either way; whether it could SAVE it is `unsaved`, below.
        if (cmd === "pause") this.pausedPairs.add(args?.pair);
        else this.pausedPairs.delete(args?.pair);
        const reply = this.status(args?.pair);
        if (this.unsaved) reply.response.pause_unsaved = this.unsaved;
        return reply;
      }
      case "restart_service":
        // A restart that worked puts the daemon on the file's settings: it runs the file's folders now.
        if (this.restartEnding === "restarted") {
          this.pairUnknown = null;
          if (this.roster && !this.refuseToList) this.names = [...this.roster];
        }
        return {
          ending: this.restartEnding,
          reason: this.restartEnding === "restarted" ? undefined : "it would not stop",
        };
      case "resync":
        return { ...this.status(args?.pair), error: this.resyncError };
      case "choose_folder":
        return this.picked;
      case "path_sync_status":
        return { tracked: false };
      case "approve":
        return {
          state: "idle",
          error: null,
          response: {
            paused: false,
            message: "approved 1 pending deletion(s); run `proton-sync syncnow` to apply now",
          },
        };
      case "keep":
        return {
          state: "idle",
          error: null,
          response: {
            paused: false,
            message: "kept 1 pending deletion(s); the other side is put back on the next sync",
          },
        };
      case "run_dry_run":
        return this.planFor(args?.pair);
      case "apply_plan":
        return { state: "applied", apply_seq: 1, executed: 1, skipped_destructive: 0, failed: 0 };
      case "sync_now":
        return this.status();
      default:
        return null;
    }
  }
}

const { server, port } = await serve();
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const failures = [];

/**
 * A fresh page wired to a fresh bridge. `query` is the page's own (`?surface=tray` opens the tray panel).
 *
 * `restore` is a LAUNCH AFTER ANOTHER: `{ state, skewMs }` puts back the notifier state a previous page saved
 * (this function clears it, so each scenario starts as a first launch) and moves the page's clock on, so that
 * only the notifier's memory — not the 30-second window — can hold a banner back.
 */
async function open(bridge, query = "", restore = null) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1040, height: 764, deviceScaleFactor: 1 });
  await page.exposeFunction("__bridge", (cmd, args) => bridge.handle(cmd, args));
  await page.evaluateOnNewDocument(() => {
    window.__listeners = {};
    // THE NOTIFIER'S MEMORY LIVES IN localStorage, which every page of this browser shares. A scenario that
    // raises a banner must not leave `said` and `lastAt` for the next page to be silenced by, so each document
    // starts without it (the app writes it back on its first poll, as it does on a first launch).
    try {
      localStorage.removeItem("notifier");
    } catch (_) {
      /* a page with no storage has nothing to clear */
    }
    // The page's clock, with a skew a scenario can move: the notifier measures "nothing has synced for a day"
    // against `Date.now()`, and a scenario about a daemon that STOPPED needs time to pass while it is down.
    // Zero unless one moves it, so every other scenario reads the clock it always did.
    window.__skewMs = 0;
    const realNow = Date.now.bind(Date);
    Date.now = () => realNow() + window.__skewMs;
    // The injection Tauri makes, as an object literal: the lint rule that keeps the app's own code
    // on the `api` facade matches the dotted `window.__TAURI__` and this is the one place the
    // injection is MADE rather than reached for.
    Object.assign(window, {
      __TAURI__: {
        core: { invoke: (cmd, args) => window.__bridge(cmd, args ?? null) },
        event: {
          listen: (name, callback) => {
            window.__listeners[name] = callback;
            return Promise.resolve(() => {});
          },
        },
      },
    });
  });
  if (restore) {
    // Registered AFTER the clearing script above, and scripts run in order: this is what the page finds.
    await page.evaluateOnNewDocument(
      (saved, skew) => {
        localStorage.setItem("notifier", saved);
        window.__skewMs = skew;
      },
      restore.state,
      restore.skewMs,
    );
  }
  page.on("pageerror", (error) => failures.push(`page error: ${error.message}`));
  await page.goto(`http://127.0.0.1:${port}/index.html${query}`, { waitUntil: "networkidle0" });
  return page;
}

/**
 * Wait until the page has PROCESSED every reply the bridge has already sent it. A call made now is
 * answered after them, and replies reach the page in the order they were sent, so by the time this
 * one's answer is back each earlier reply's continuation has run — the store has moved and the
 * screen has been drawn. It is a condition, where a sleep was a guess at how long that takes.
 */
const settle = (page) => page.evaluate(() => window.__bridge("fence", null));

/** Is there a button reading exactly `label`? */
const hasButton = (page, label) =>
  page.evaluate(
    (text) => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === text),
    label,
  );

/**
 * One poll, forced and awaited: the page's own `pair-selected` listener polls at once, which is the
 * daemon's way of saying "look again" and the only way to make the page ask without waiting two seconds.
 */
async function poll(page, bridge) {
  const before = bridge.called("get_status").length;
  await page.evaluate(() => window.__listeners["pair-selected"]({ payload: null }));
  await until("a poll", () => bridge.called("get_status").length > before);
  await settle(page);
}

const pageText = (page) => page.evaluate(() => document.getElementById("app-root")?.innerText ?? "");

/** Click the first button whose text is exactly `text`; waits for it to exist. */
async function press(page, text) {
  await until(`a button reading "${text}"`, () =>
    page.evaluate(
      (label) => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === label),
      text,
    ),
  );
  await page.evaluate((label) => {
    [...document.querySelectorAll("button")].find((b) => b.textContent.trim() === label).click();
  }, text);
}

/** The selection moves: the bridge answers for the other pair, and the `pair-selected` event fires. */
async function select(page, bridge, pair) {
  bridge.selected = pair;
  const before = bridge.called("get_status").length;
  await page.evaluate((name) => window.__listeners["pair-selected"]({ payload: name }), pair);
  await until("a status poll after the switch", () => bridge.called("get_status").length > before);
  await settle(page); // the reply lands, the store moves, the screen re-renders
}

async function scenario(name, run) {
  const before = failures.length;
  try {
    await run();
  } catch (error) {
    failures.push(`${name}: ${error.message}`);
  }
  if (failures.length === before) console.log(`fidelity:pairs — ${name}: ok`);
}

function expectPair(calls, pair, what) {
  if (calls.length === 0) throw new Error(`${what}: nothing was sent`);
  const wrong = calls.filter((call) => call.args?.pair !== pair);
  if (wrong.length) {
    throw new Error(
      `${what}: sent for ${JSON.stringify(wrong.map((c) => c.args?.pair))}, expected ${JSON.stringify(pair)} ` +
        `(the selection was ${JSON.stringify(wrong.map((c) => c.selectedThen))} when it arrived)`,
    );
  }
}

// ---- 1. the tail of a decision ---------------------------------------------------------------------
await scenario("an approval's follow-up goes to the pair the approval was for", async () => {
  const bridge = new Bridge({ docs: [deletion("a.txt")], photos: [] });
  const page = await open(bridge);
  await press(page, MAIN.band.deletionAction); // the band's `Review`
  const release = bridge.hold("approve");
  await press(page, DELETIONS.toTrash);
  await until("the approval", () => bridge.called("approve").length === 1);
  expectPair(bridge.called("approve"), "docs", "approve");

  await select(page, bridge, "photos"); // while the approval is still in flight
  release();
  await until("the sync nudge", () => bridge.called("sync_now").length === 1);
  expectPair(bridge.called("sync_now"), "docs", "sync_now after the switch");
  await page.close();
});

// ---- 2. the press of `Run this sync` ---------------------------------------------------------------
await scenario(
  "a press of Run this sync applies the plan it reviewed, to the pair it was made for",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await press(page, "Plan a sync");
    await until("the rehearsal", () => bridge.called("run_dry_run").length === 1);
    expectPair(bridge.called("run_dry_run"), "docs", "run_dry_run");
    await until("the gate", () => page.$(".delete-gate"));
    await page.type(".delete-gate", "DELETE");

    const release = bridge.hold("approve");
    await press(page, PLAN.run);
    await until("the first approval", () => bridge.called("approve").length === 1);
    expectPair(bridge.called("approve"), "docs", "the plan's deletion approval");

    await select(page, bridge, "photos"); // resets the plan screen, which is rehearsing for photos now
    release();
    await until("the apply", () => bridge.called("apply_plan").length === 1);
    expectPair(bridge.called("apply_plan"), "docs", "apply_plan after the switch");
    const applied = bridge.called("apply_plan")[0].args;
    if (applied.token !== "token-for-docs") {
      throw new Error(`apply_plan carried ${JSON.stringify(applied.token)}, not the plan that was reviewed`);
    }
    await page.close();
  },
);

// ---- 3. a switch rebuilds the cards ------------------------------------------------------------------
await scenario("after a switch, Keep on the card now showing goes to the pair now showing", async () => {
  // The SAME path at the SAME fingerprint in both folders: the two queues draw identical cards.
  const same = deletion("same.txt");
  const bridge = new Bridge({ docs: [same], photos: [same] });
  const page = await open(bridge);
  await press(page, MAIN.band.deletionAction);
  await until("the card", () =>
    page.evaluate(
      (label) => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === label),
      DELETIONS.keepLocal,
    ),
  );
  await select(page, bridge, "photos");
  await press(page, DELETIONS.keepLocal);
  await until("the keep", () => bridge.called("keep").length === 1);
  expectPair(bridge.called("keep"), "photos", "keep");
  await page.close();
});

// ---- 4. the hero's buttons are the hero's -----------------------------------------------------------
await scenario("after a switch, the hero's Pause pauses the pair the hero is now about", async () => {
  // Same hero for both pairs (settled), so the screen is PATCHED across the switch and its buttons are
  // not rebuilt. A button bound when it was built would go on acting on the pair it was built for.
  const bridge = new Bridge({ docs: [], photos: [] });
  const page = await open(bridge);
  // At two folders the button names its folder (decision D11), so the hero built for `docs` says so and
  // the one the switch patches into must say `photos` — a stale label over a live handler would be a
  // button that reads `Pause docs` and pauses photos.
  await until("the settled hero", () => hasButton(page, TRAY.pausePair("docs")));
  await select(page, bridge, "photos");
  await until("the hero named for photos", () => hasButton(page, TRAY.pausePair("photos")));
  if (await hasButton(page, TRAY.pausePair("docs"))) {
    throw new Error("after the switch the hero still offers to pause docs");
  }
  await press(page, TRAY.pausePair("photos"));
  await until("the pause", () => bridge.called("pause").length === 1);
  expectPair(bridge.called("pause"), "photos", "pause");
  await page.close();
});

// ---- 5. a pair that has never synced --------------------------------------------------------------
await scenario(
  "at two pairs a never-synced pair is drawn as waiting for its turn, and the first-run takeover stays shut",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { neverSynced: ["docs"] });
    const page = await open(bridge);
    await until("the hero", async () => (await pageText(page)).includes(TRAY.startingTitle));
    const shown = await pageText(page);
    if (shown.includes(MAIN.settled))
      throw new Error(`a pair that has never synced was drawn as "${MAIN.settled}"`);
    if (!shown.includes(TRAY.startingSub("docs")))
      throw new Error("the second sentence of the starting hero is missing");
    // The takeover is the full-window wizard, which draws its own chip and no ⋯ menu.
    if (/step 1 of 2/.test(shown)) throw new Error("the first-run takeover opened over a running app");
    await page.close();
  },
);

await scenario("at one pair the same reply still opens the first-run takeover", async () => {
  // The other half of the rule: it is the COUNT that keeps the wizard shut, not the state.
  const bridge = new Bridge({ docs: [] }, { names: ["docs"], neverSynced: ["docs"] });
  const page = await open(bridge);
  await until("the takeover", async () => /step 1 of 2/.test(await pageText(page)));
  await page.close();
});

// ---- 7. a late read ---------------------------------------------------------------------------------
await scenario(
  "a late conflict read for the pair that was left never fills the card now showing",
  async () => {
    const bridge = new Bridge(
      { docs: [], photos: [] },
      { conflicts: { docs: [CONFLICT], photos: [CONFLICT] } },
    );
    const releaseDocs = bridge.hold("read_conflict_pair", "docs");
    const releasePhotos = bridge.hold("read_conflict_pair", "photos");
    const page = await open(bridge);
    await press(page, MAIN.band.conflictAction);
    await until("docs' read", () => bridge.called("read_conflict_pair").some((c) => c.args?.pair === "docs"));

    await select(page, bridge, "photos"); // while docs' read is still out
    await until("photos' read", () =>
      bridge.called("read_conflict_pair").some((c) => c.args?.pair === "photos"),
    );

    releaseDocs(); // the OLDER read answers first, after the selection has moved on
    await settle(page);
    releasePhotos();
    await settle(page);
    await press(page, CONFLICTS.showDiff);
    const shown = await pageText(page);
    if (/MARK-docs/.test(shown)) {
      throw new Error("the card for photos shows docs' file content, read for a different folder");
    }
    // The positive control: the card is not merely blank, it has photos' own content.
    if (!/MARK-photos-mine/.test(shown))
      throw new Error("the card for photos does not show photos' own content");
    await page.close();
  },
);

// ---- 8. the decision on a conflict ----------------------------------------------------------------------
await scenario("a decision on a conflict goes to the pair the card was drawn for", async () => {
  const bridge = new Bridge(
    { docs: [], photos: [] },
    { conflicts: { docs: [CONFLICT], photos: [CONFLICT] } },
  );
  const page = await open(bridge);
  await press(page, MAIN.band.conflictAction);
  const keepBoth = await until("the card's choices", async () => {
    const handle = await page.evaluateHandle(
      (label) =>
        [...document.querySelectorAll("button")].find(
          (b) => b.querySelector(".btn-choice-name")?.textContent.trim() === label,
        ) ?? null,
      CONFLICTS.keepBoth,
    );
    return handle.asElement();
  });
  // The selection moves under a card that has not been redrawn yet, and the press lands on THAT card:
  // the button is held across the switch, which is what a click arriving in the gap looks like.
  await select(page, bridge, "photos");
  await keepBoth.evaluate((button) => button.click());
  await until("the decision", () => bridge.called("resolve_conflict").length === 1);
  expectPair(bridge.called("resolve_conflict"), "docs", "resolve_conflict");
  await page.close();
});

await scenario(
  "a late decision on one pair's conflict leaves the card another pair is showing where it was",
  async () => {
    // docs has one conflict and photos two. The decision on docs' is out when the selection moves to
    // photos, and photos' queue is stepped to its SECOND card. The decision then lands: it must settle
    // docs' conflict and touch nothing the photos screen holds. Unguarded, its continuation set the
    // position from docs' rescan (one conflict, so index 0) and the photos screen jumped back to its first.
    const SECOND = { original: "other.txt", sidecar: "other.proton-cloud.txt", kind: "content" };
    const bridge = new Bridge(
      { docs: [], photos: [] },
      { conflicts: { docs: [CONFLICT], photos: [CONFLICT, SECOND] } },
    );
    const releaseDecision = bridge.hold("resolve_conflict");
    const page = await open(bridge);
    await press(page, MAIN.band.conflictAction);
    const keepBoth = await until("the card's choices", async () => {
      const handle = await page.evaluateHandle(
        (label) =>
          [...document.querySelectorAll("button")].find(
            (b) => b.querySelector(".btn-choice-name")?.textContent.trim() === label,
          ) ?? null,
        CONFLICTS.keepBoth,
      );
      return handle.asElement();
    });
    await keepBoth.evaluate((button) => button.click());
    await until("the decision to go out", () => bridge.called("resolve_conflict").length === 1);
    await select(page, bridge, "photos"); // while the decision is still out
    await press(page, "\u203a"); // `›`: photos' second card
    await until("photos' second card", async () => /other\.txt/.test(await pageText(page)));

    releaseDecision();
    await settle(page);
    await until("the rescan of docs", () =>
      bridge.called("scan_conflicts").some((c) => c.args?.pair === "docs"),
    );
    await settle(page);
    const shown = await pageText(page);
    if (!/other\.txt/.test(shown)) {
      throw new Error("the late decision for docs moved the photos screen off its second card");
    }
    expectPair(bridge.called("resolve_conflict"), "docs", "resolve_conflict");
    await page.close();
  },
);

// ---- 9. the tray panel's pin ------------------------------------------------------------------------------
await scenario("the tray panel shows the pair its rows act on, not the one the window selected", async () => {
  // The window has selected `photos`. The panel's rows act on the default pair (`docs`), which has never
  // synced here: a panel that followed the selection would say `Everything is up to date` beside rows
  // that act on a folder it is not describing.
  const bridge = new Bridge({ docs: [], photos: [] }, { selected: "photos", neverSynced: ["docs"] });
  const page = await open(bridge, "?surface=tray");
  // A poll of EITHER kind, so a panel that went back to `get_status` fails the assertion below with
  // its own message rather than timing out waiting for a command it no longer sends.
  const polls = () => bridge.calls.filter((call) => call.cmd === "tray_status" || call.cmd === "get_status");
  await until("the first poll", () => polls().length >= 1);
  await settle(page);
  const before = polls().length;
  await page.evaluate(() => window.__listeners["pair-selected"]({ payload: "photos" }));
  await until("a poll after the selection moved", () => polls().length > before);
  await settle(page);

  // THE FIRST POLL TOO. It is a command with no pair to name, so it cannot mean the selection; and the
  // panel never asks `get_status`, whose unnamed read IS the selection.
  const wrong = polls().filter((call) => call.cmd !== "tray_status" || call.args != null);
  if (wrong.length) {
    throw new Error(
      `the panel asked ${JSON.stringify(wrong.map((c) => [c.cmd, c.args]))} instead of a bare tray_status`,
    );
  }
  const shown = await pageText(page);
  if (!shown.includes(TRAY.startingSub("docs"))) {
    throw new Error(
      `the panel does not describe the default pair (docs, not yet started): ${JSON.stringify(shown)}`,
    );
  }
  if (shown.includes(MAIN.compact.upToDate))
    throw new Error("the panel drew the selected pair (photos) as up to date");
  await page.close();
});

// ---- 10. the tray panel's rows at two folders --------------------------------------------------------------
await scenario(
  "the tray panel at two folders names the worst one and a folder row goes to its folder",
  async () => {
    // `photos` failed; `docs` (the default, the folder the reply describes) is fine.
    const bridge = new Bridge(
      { docs: [], photos: [] },
      {
        selected: "docs",
        states: { photos: { state: "failed", rank: 5, summary: { last_error: "boom", pending_changes: 3 } } },
      },
    );
    const page = await open(bridge, "?surface=tray");
    await until("the first poll", () => bridge.called("tray_status").length >= 1);
    await settle(page);

    const shown = await pageText(page);
    if (!shown.includes("photos"))
      throw new Error(`the panel does not name the worst folder: ${JSON.stringify(shown)}`);
    if (!shown.includes(MAIN.failed))
      throw new Error(`the panel is not the failed folder's: ${JSON.stringify(shown)}`);
    if (shown.includes(MAIN.compact.upToDate))
      throw new Error("the panel drew an up-to-date folder over a failed one");
    if (shown.includes("boom")) throw new Error("the daemon's own sentence reached a 362px panel");
    if (shown.includes(TRAY.pause) || shown.includes(TRAY.resume)) {
      throw new Error(`a row says ${TRAY.pause}/${TRAY.resume} at two folders: ${JSON.stringify(shown)}`);
    }

    await press(page, TRAY.pausePair("photos"));
    await until("the row to go out", () => bridge.called("tray_action").length === 1);
    const sent = bridge.called("tray_action")[0].args;
    if (sent?.id !== "pause@photos") {
      throw new Error(
        `pressing "${TRAY.pausePair("photos")}" sent ${JSON.stringify(sent)}, not pause@photos`,
      );
    }
    // And the reply the panel is handed is the default folder's own status, so the panel still shows what
    // it showed — the failed folder — rather than repainting as the folder the row was for.
    await settle(page);
    const after = await pageText(page);
    if (!after.includes("photos") || !after.includes(MAIN.failed)) {
      throw new Error(`the panel changed after a folder's row: ${JSON.stringify(after)}`);
    }
    await page.close();
  },
);

// ---- 11. the tray panel's Review ---------------------------------------------------------------------------
await scenario(
  "the tray panel's Review names the folder that holds the decisions, and follows them",
  async () => {
    // Both folders are fine; `photos` (the SECOND folder, not the one the reply describes) holds a deletion.
    const bridge = new Bridge({ docs: [], photos: [deletion("a.txt")] }, { selected: "docs" });
    const page = await open(bridge, "?surface=tray");
    await until("the first poll", () => bridge.called("tray_status").length >= 1);
    await settle(page);

    // The folder line, not the page text: the menu names both folders whatever the panel is about.
    const folderLine = () =>
      page.evaluate(() => document.querySelector(".compact-pair")?.textContent ?? null);
    const shown = await pageText(page);
    if (shown.includes(MAIN.compact.upToDate)) {
      throw new Error(
        `a deletion waiting in the other folder was hidden behind "Up to date": ${JSON.stringify(shown)}`,
      );
    }
    if (!shown.includes(MAIN.compact.needYou(1)) || (await folderLine()) !== "photos") {
      throw new Error(`the panel is not photos' needs-you panel: ${JSON.stringify(shown)}`);
    }

    await press(page, MAIN.compact.review);
    await until("Review to go out", () => bridge.called("tray_action").length === 1);
    const first = bridge.called("tray_action")[0].args;
    if (first?.id !== "review@photos") {
      throw new Error(`Review sent ${JSON.stringify(first)}, not review@photos`);
    }

    // The decision moves to `docs` while the panel is still a needs-you panel: the next poll PATCHES it
    // (same form, same rows), and the button it keeps was built for photos.
    bridge.queues = { docs: [deletion("b.txt")], photos: [] };
    await until("the panel to follow the decision", async () => {
      await settle(page);
      return (await folderLine()) === "docs";
    });
    await press(page, MAIN.compact.review);
    await until("the second Review to go out", () => bridge.called("tray_action").length === 2);
    const second = bridge.called("tray_action")[1].args;
    if (second?.id !== "review@docs") {
      throw new Error(
        `after the decision moved to docs, Review sent ${JSON.stringify(second)}, not review@docs ` +
          "(the patched panel kept the button it was built with)",
      );
    }
    await page.close();
  },
);

// ---- 12-14. the Settings screen's staged edits -------------------------------------------------------------

/** Every policy card on the Deletions tab, by title: whether it is the selected one. */
const policyCards = (page) =>
  page.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll(".settings-cards .radio-card")].map((card) => [
        card.querySelector(".radio-title")?.textContent.trim(),
        card.getAttribute("aria-checked") === "true",
      ]),
    ),
  );

/** Open Settings on its Deletions tab and wait for the cards to be drawn from a config that has arrived. */
async function openDeletions(page) {
  await press(page, "Settings");
  await press(page, SETTINGS.tabs.deletions);
  await until("the policy cards", async () => Object.values(await policyCards(page)).some(Boolean));
}

const pressCard = (page, title) =>
  page.evaluate((label) => {
    [...document.querySelectorAll(".settings-cards .radio-card")]
      .find((card) => card.querySelector(".radio-title")?.textContent.trim() === label)
      .click();
  }, title);

/** The two folders' saved policies differ, so a card drawn from the wrong folder's file is a different card. */
const SETTINGS_CONFIGS = {
  docs: { deletion_policy: "ask_every_time" },
  photos: { deletion_policy: "only_permanent" },
};

await scenario(
  "a save of staged settings names the pair the edit was staged for, not the one selected when it runs",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS });
    const page = await open(bridge);
    await openDeletions(page);
    await pressCard(page, SETTINGS.askNever); // staged against docs
    const save = await until("an armed Save", async () => {
      const handle = await page.evaluateHandle(
        (label) =>
          [...document.querySelectorAll("button")].find(
            (b) => b.textContent.trim() === label && !b.disabled,
          ) ?? null,
        SETTINGS.save,
      );
      return handle.asElement();
    });

    // The selection moves under a Save that has not been redrawn yet, and the press lands on THAT
    // button: held across the switch, which is what a click arriving in the gap looks like.
    await select(page, bridge, "photos");
    await save.evaluate((button) => button.click());
    await until("the write", () => bridge.called("write_config").length === 1);
    expectPair(bridge.called("write_config"), "docs", "write_config");
    const sent = bridge.called("write_config")[0].args.update;
    if (JSON.stringify(sent) !== JSON.stringify({ deletion_policy: "never" })) {
      throw new Error(`write_config carried ${JSON.stringify(sent)}, not docs' staged policy`);
    }
    await page.close();
  },
);

await scenario("what is staged for one pair is never shown on, or saved for, another", async () => {
  const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS });
  const page = await open(bridge);
  await openDeletions(page);
  await pressCard(page, SETTINGS.askNever); // staged against docs
  const staged = await policyCards(page);
  if (!staged[SETTINGS.askNever]) throw new Error("the staged card is not drawn as chosen for docs");

  await select(page, bridge, "photos");
  // Photos' config arrives on the next read; wait until the card drawn is photos' own saved one.
  const photos = await until("photos' own policy", async () => {
    const cards = await policyCards(page);
    return cards[SETTINGS.askPermanent] ? cards : null;
  });
  if (photos[SETTINGS.askNever]) throw new Error("docs' staged policy is drawn as photos'");
  const armed = await page.evaluate(
    (label) =>
      [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === label && !b.disabled),
    SETTINGS.save,
  );
  if (armed) throw new Error("photos has nothing staged but Save is armed: docs' edit leaks into it");

  // Nothing was lost by looking at the other folder: docs' edit is where it was left.
  await select(page, bridge, "docs");
  const back = await until("docs' staged policy", async () => {
    const cards = await policyCards(page);
    return cards[SETTINGS.askNever] ? cards : null;
  });
  if (!back[SETTINGS.askNever]) throw new Error("docs' staged policy was lost by looking at photos");

  await press(page, SETTINGS.save);
  await until("the write", () => bridge.called("write_config").length >= 1);
  await settle(page);
  expectPair(bridge.called("write_config"), "docs", "write_config");
  if (bridge.called("write_config").length !== 1) {
    throw new Error(`${bridge.called("write_config").length} writes went out for one folder's edit`);
  }
  await page.close();
});

await scenario(
  "the settings read asks about the folder on screen and is filed under the one it names",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS, selected: "photos" });
    const page = await open(bridge);
    await openDeletions(page);
    // The first read may name nothing (no status has said which folder is shown); every read after the
    // status has landed names the folder on screen, and that folder's saved policy is what is drawn.
    await until("a read that names photos", () =>
      bridge.called("read_config").some((call) => call.args?.pair === "photos"),
    );
    const cards = await until("photos' policy", async () => {
      const drawn = await policyCards(page);
      return drawn[SETTINGS.askPermanent] ? drawn : null;
    });
    if (cards[SETTINGS.askEvery]) throw new Error("another folder's saved policy is drawn for photos");
    const named = bridge.called("read_config").filter((call) => call.args?.pair != null);
    const wrong = named.filter((call) => call.args.pair !== "photos");
    if (wrong.length) {
      throw new Error(`a read named ${JSON.stringify(wrong.map((c) => c.args.pair))} while photos was shown`);
    }
    await page.close();
  },
);

await scenario("the Details dialog reads the config of the folder on screen", async () => {
  const bridge = new Bridge(
    { docs: [], photos: [] },
    {
      configs: {
        docs: { scan_interval_secs: 300 },
        photos: { scan_interval_secs: 777 },
      },
    },
  );
  const page = await open(bridge);
  await until("docs' config", () => bridge.called("read_config").length >= 1);
  await select(page, bridge, "photos");
  await until("a read of photos' config", () =>
    bridge.called("read_config").some((call) => call.args?.pair === "photos"),
  );
  await settle(page);
  await press(page, "Details");
  const shown = await until("the interval row", async () => {
    const text = await pageText(page);
    return /scan_interval/.test(text) ? text : null;
  });
  if (!shown.includes("777s")) {
    throw new Error(`the Details dialog does not state photos' interval: ${JSON.stringify(shown)}`);
  }
  if (shown.includes("300s")) throw new Error("the Details dialog states docs' interval over photos");
  await page.close();
});

// ---- 16-24. the folder selector and what the window says about two folders (#102 phase 5c-1) ----------

await scenario(
  "the selector is drawn at two folders and at no fewer, and the hero's buttons follow it",
  async () => {
    const bridge = new Bridge({ docs: [] }, { names: ["docs"] });
    const page = await open(bridge);
    await until("the settled hero", () => hasButton(page, MAIN.pause));
    if (await page.$(".pair-select")) throw new Error("a pill was drawn for ONE folder (decision D2)");
    if (await hasButton(page, TRAY.pausePair("docs")))
      throw new Error("the one-folder hero names its folder");

    // A second folder arrives with the next poll: the header has to GROW the pill, not keep the one it had.
    bridge.names = ["docs", "photos"];
    bridge.queues = { docs: [], photos: [] };
    await poll(page, bridge);
    await until("the pill", () => page.$(".pair-select .pair-pill"));
    const named = await page.evaluate(() => document.querySelector(".pair-pill-name")?.textContent);
    if (named !== "docs") throw new Error(`the pill names ${JSON.stringify(named)}, not the folder shown`);
    await until("the hero naming its folder", () => hasButton(page, TRAY.pausePair("docs")));
    if (await hasButton(page, MAIN.pause))
      throw new Error("two folders, and the hero's button still says plain `Pause`");

    // …and when it goes again the header shrinks back to the one-folder header.
    bridge.names = ["docs"];
    await poll(page, bridge);
    await until("the pill to go", async () => !(await page.$(".pair-select")));
    await until("the one-folder hero", () => hasButton(page, MAIN.pause));
    await page.close();
  },
);

await scenario(
  "the folder list opens by keyboard and the poll leaves the keyboard where it was",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await until("the pill", () => page.$(".pair-pill"));
    await settle(page);
    const focused = () =>
      page.evaluate(() => {
        const node = document.activeElement;
        return node?.classList.contains("pair-pill") ? "pill" : (node?.dataset?.pair ?? null);
      });

    // THE PILL SURVIVES A POLL. A header rebuilt on the tick is a different node, and the keyboard is on the
    // old one: it falls to <body> inside two seconds, which is the failure the patching discipline exists for.
    await page.focus(".pair-pill");
    await page.evaluate(() => {
      document.querySelector(".pair-pill").__probe = "the same node";
    });
    await poll(page, bridge);
    await poll(page, bridge);
    const survived = await page.evaluate(
      () =>
        document.activeElement?.__probe === "the same node" &&
        document.activeElement.classList.contains("pair-pill"),
    );
    if (!survived) throw new Error("the pill was rebuilt by a poll: the keyboard is no longer on it");

    // Down opens the list ON THE FOLDER THAT IS CHOSEN, and the arrows walk it.
    await page.keyboard.press("ArrowDown");
    await until("the list", () => page.$(".pair-popover"));
    await until("the keyboard in the list", async () => (await focused()) === "docs");
    await page.keyboard.press("ArrowDown");
    if ((await focused()) !== "photos") throw new Error(`ArrowDown moved to ${await focused()}, not photos`);

    // THE LIST SURVIVES A POLL TOO: three of them, and the row the keyboard is on is still the same one.
    await page.evaluate(() => {
      document.querySelector('.pair-row[data-pair="photos"]').__probe = "the same row";
    });
    await poll(page, bridge);
    await poll(page, bridge);
    await poll(page, bridge);
    const rowSurvived = await page.evaluate(
      () =>
        document.activeElement?.__probe === "the same row" &&
        Boolean(document.querySelector(".pair-popover")),
    );
    if (!rowSurvived) {
      throw new Error(
        "a poll rebuilt the folder list under the keyboard (the row is gone, or the list closed)",
      );
    }

    await page.keyboard.press("Home");
    if ((await focused()) !== "docs") throw new Error("Home did not go to the first folder");
    await page.keyboard.press("End");
    if ((await focused()) !== "photos") throw new Error("End did not go to the last folder");

    // Esc closes it and gives the keyboard back to the pill — and does NOT also leave the screen under it.
    await page.keyboard.press("Escape");
    await until("the list to close", async () => !(await page.$(".pair-popover")));
    if ((await focused()) !== "pill") throw new Error("Esc did not return the keyboard to the pill");

    // Enter on a row chooses it: Rust is told, the list closes, and the window follows.
    await page.keyboard.press("ArrowDown");
    await until("the list again", async () => (await focused()) === "docs");
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await until("select_pair", () => bridge.called("select_pair").length === 1);
    if (bridge.called("select_pair")[0].args?.name !== "photos") {
      throw new Error(`select_pair carried ${JSON.stringify(bridge.called("select_pair")[0].args)}`);
    }
    await settle(page);
    await until("the window on photos", () => hasButton(page, TRAY.pausePair("photos")));
    const named = await page.evaluate(() => document.querySelector(".pair-pill-name")?.textContent);
    if (named !== "photos") throw new Error(`the pill names ${named} after choosing photos`);
    if (await page.$(".pair-popover")) throw new Error("the list stayed open after a choice");

    // Choosing the folder already shown asks Rust for nothing.
    await page.keyboard.press("ArrowDown");
    await until("the list once more", async () => (await focused()) === "photos");
    await page.keyboard.press("Enter");
    await settle(page);
    if (bridge.called("select_pair").length !== 1)
      throw new Error("choosing the folder already shown sent select_pair");

    // A press outside closes it.
    await page.focus(".pair-pill");
    await page.keyboard.press("ArrowDown");
    await until("the list a third time", () => page.$(".pair-popover"));
    await page.mouse.click(520, 600);
    await until("the list to close on a press elsewhere", async () => !(await page.$(".pair-popover")));
    await page.close();
  },
);

await scenario("a status reply for the folder that was left never takes the window back to it", async () => {
  const bridge = new Bridge({ docs: [], photos: [] });
  const page = await open(bridge);
  await until("the pill", () => page.$(".pair-pill"));
  await settle(page);

  // A poll leaves NOW, about docs, and is answered only after the person has moved on.
  const releaseDocs = bridge.holdLate("get_status");
  const before = bridge.called("get_status").length;
  await page.evaluate(() => window.__listeners["pair-selected"]({ payload: null }));
  await until("the poll to leave", () => bridge.called("get_status").length > before);

  await page.focus(".pair-pill");
  await page.keyboard.press("ArrowDown");
  await until("the list", () => page.$(".pair-row"));
  await page.evaluate(() => document.querySelector('.pair-row[data-pair="photos"]').click());
  await until("select_pair", () => bridge.called("select_pair").length === 1);
  await until("the window on photos", () => hasButton(page, TRAY.pausePair("photos")));

  releaseDocs(); // the OLD reply lands, saying docs is selected
  await settle(page);
  await settle(page);
  const named = await page.evaluate(() => document.querySelector(".pair-pill-name")?.textContent);
  if (named !== "photos") throw new Error(`a late reply moved the window back to ${named}`);
  if (!(await hasButton(page, TRAY.pausePair("photos"))))
    throw new Error("the hero went back to the folder that was left");
  await page.close();
});

await scenario(
  "a switch disarms a typed-DELETE gate and does not carry it onto the other folder's row",
  async () => {
    // The SAME permanent deletion in both folders, so the row the switch lands on could answer to the
    // gate that was armed for the other one.
    const same = { ...deletion("same.txt"), direction: "local", disposal: "permanent" };
    const bridge = new Bridge({ docs: [same], photos: [same] });
    const page = await open(bridge);
    await press(page, MAIN.band.deletionAction);
    await until("the typed gate", () => page.$(".delete-gate"));
    await page.type(".delete-gate", "DELETE");
    await press(page, DELETIONS.delete);
    await until("the armed takeover", () => hasButton(page, DELETIONS.armedConfirm));
    await select(page, bridge, "photos");
    if (await hasButton(page, DELETIONS.armedConfirm)) {
      throw new Error("the gate armed for docs is armed on photos' identical row");
    }
    // The positive control: photos' own card is there, un-armed.
    await until("photos' own card", () => hasButton(page, DELETIONS.delete));
    await page.close();
  },
);

await scenario("the footer line names the folder on screen", async () => {
  const bridge = new Bridge({ docs: [], photos: [] });
  const page = await open(bridge);
  await until("docs' line", async () =>
    (await pageText(page)).includes(MAIN.footerPair("/home/u/docs", "/Drive/docs")),
  );
  await select(page, bridge, "photos");
  await until("photos' line", async () =>
    (await pageText(page)).includes(MAIN.footerPair("/home/u/photos", "/Drive/photos")),
  );
  if ((await pageText(page)).includes("/home/u/docs"))
    throw new Error("docs' root is on screen under photos");
  await page.close();
});

await scenario(
  "the ring is on the pill when another folder is waiting, and the chip stays the selected folder's",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [deletion("a.txt")] });
    const page = await open(bridge);
    await until("the ring", () => page.$(".pair-pill .pair-pill-marker"));
    const chip = () => page.evaluate(() => document.querySelector("[data-variant]")?.textContent.trim());
    if ((await chip()) !== CHROME.chips.idle) {
      throw new Error(`the chip counts another folder's queue: ${JSON.stringify(await chip())}`);
    }
    // The list says which folder, and how many — a count, in the list and never on the pill.
    await page.focus(".pair-pill");
    await page.keyboard.press("ArrowDown");
    await until("the list", () => page.$(".pair-row"));
    const counts = await page.evaluate(() =>
      Object.fromEntries(
        [...document.querySelectorAll(".pair-row")].map((row) => [
          row.dataset.pair,
          row.querySelector(".pair-row-count").textContent,
        ]),
      ),
    );
    if (counts.photos !== CHROME.chips.waiting(1) || counts.docs !== "") {
      throw new Error(`the list's counts are ${JSON.stringify(counts)}`);
    }
    if (/\d/.test(await page.evaluate(() => document.querySelector(".pair-pill").textContent))) {
      throw new Error("a number is on the pill — the ring is the marker, never a count");
    }
    await page.keyboard.press("Escape");

    // Photos on screen: its own queue is the chip's, and the ring is for docs, which waits for nothing.
    await select(page, bridge, "photos");
    await until("photos' chip", async () => (await chip()) === CHROME.chips.waiting(1));
    if (await page.$(".pair-pill .pair-pill-marker"))
      throw new Error("the ring is on the pill for the folder's own queue");
    await page.close();

    // A conflict waiting in the other folder rings it too — found by the scan the poll runs for it.
    const second = new Bridge({ docs: [], photos: [] }, { conflicts: { photos: [CONFLICT] } });
    const other = await open(second);
    await until("the scan of photos", () =>
      second.called("scan_conflicts").some((c) => c.args?.pair === "photos"),
    );
    await until("the ring for a conflict", () => other.$(".pair-pill .pair-pill-marker"));
    await other.close();
  },
);

/** Which form the pill's marker is in right now — `"problem"`, `"decision"` (the ring) — or null for none. */
const markerForm = (page) =>
  page.evaluate(() => document.querySelector(".pair-pill .pair-pill-marker")?.dataset.marker ?? null);

/** The chip's text: the SELECTED folder's state, whatever the pill says about the others. */
const chipText = (page) =>
  page.evaluate(() => document.querySelector("[data-variant] .chip-text")?.textContent.trim() ?? null);

/** Open the folder list from the keyboard and wait for its rows. */
async function openList(page) {
  await page.focus(".pair-pill");
  await page.keyboard.press("ArrowDown");
  await until("the list", () => page.$(".pair-row"));
}

/** Every row of the open list by folder: `[the state word, the count]`. */
const listRows = (page) =>
  page.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll(".pair-row")].map((row) => [
        row.dataset.pair,
        [row.querySelector(".pair-row-state").textContent, row.querySelector(".pair-row-count").textContent],
      ]),
    ),
  );

const FAILED = (error = "the remote listing timed out") => ({
  state: "failed",
  rank: 5,
  summary: { last_error: error },
});

await scenario(
  "another folder that failed or is unavailable marks the pill in the problem form, and a paused one does not",
  async () => {
    // FAILED: the last pass of `photos` did not finish, and the folder on screen is fine. The window shows
    // only the folder on screen, so this is the one place the failure can be seen without opening the list.
    const failing = new Bridge({ docs: [], photos: [] }, { states: { photos: FAILED() } });
    const page = await open(failing);
    await until("the problem form", async () => (await markerForm(page)) === "problem");
    // A different SHAPE from the decision ring and not only a different hue: filled where the ring is hollow.
    const look = await page.evaluate(() => {
      const style = getComputedStyle(document.querySelector(".pair-pill-marker"));
      return { fill: style.backgroundColor, stroke: style.borderTopWidth };
    });
    if (look.fill === "rgba(0, 0, 0, 0)" || look.stroke !== "0px") {
      throw new Error(`the problem form is not a solid dot: ${JSON.stringify(look)}`);
    }
    if ((await chipText(page)) !== CHROME.chips.idle) {
      throw new Error(`the chip took another folder's failure: ${JSON.stringify(await chipText(page))}`);
    }
    // What a screen reader is told, since the dot is a colour: that another folder has a PROBLEM — not
    // that something is waiting, which is the ring's sentence.
    const named = await page.evaluate(() => document.querySelector(".pair-pill").getAttribute("aria-label"));
    if (named !== "Folder docs. Another folder has a problem.") {
      throw new Error(`the pill's accessible name is ${JSON.stringify(named)}`);
    }
    // The list says which folder, in the deck's words — the pill only says that there is one.
    await openList(page);
    const failedRows = await listRows(page);
    if (failedRows.photos[0] !== CHROME.pair.states.failed) {
      throw new Error(`the list does not say photos failed: ${JSON.stringify(failedRows)}`);
    }
    await page.keyboard.press("Escape");
    // THE FOLDER ON SCREEN FAILING IS THE CHIP'S AND THE HERO'S, never the pill's.
    await select(page, failing, "photos");
    await until("photos' own chip", async () => (await chipText(page)) === "sync failed");
    if (await markerForm(page)) {
      throw new Error("the pill is marked for the failure of the folder that is on screen");
    }
    await page.close();

    // UNAVAILABLE: an unplugged drive publishes the reason as its `last_error` and keeps its last sync;
    // Rust derives `failed` from that summary, and the pill marks it the same way.
    const unplugged = new Bridge(
      { docs: [], drive: [] },
      {
        names: ["docs", "drive"],
        states: { drive: FAILED("the sync folder /mnt/usb/Sync is not available") },
      },
    );
    const other = await open(unplugged);
    await until(
      "the problem form for an unavailable folder",
      async () => (await markerForm(other)) === "problem",
    );
    await other.close();

    // PAUSED is the person's own choice: no marker of either form, and the list still says it.
    const paused = new Bridge(
      { docs: [], photos: [] },
      { states: { photos: { state: "paused", rank: 1, summary: { paused: true } } } },
    );
    const calm = await open(paused);
    await until("the pill", () => calm.$(".pair-pill"));
    await settle(calm);
    await settle(calm);
    if (await markerForm(calm)) throw new Error("the pill is marked for a folder the person paused");
    await openList(calm);
    if ((await listRows(calm)).photos[0] !== CHROME.pair.states.paused) {
      throw new Error("the list does not say photos is paused");
    }
    await calm.close();
  },
);

await scenario(
  "when a decision waits in one folder and another has failed, the pill shows the problem form",
  async () => {
    const bridge = new Bridge(
      { docs: [], photos: [deletion("a.txt")], music: [] },
      { names: ["docs", "photos", "music"], states: { music: FAILED() } },
    );
    const page = await open(bridge);
    await until("the problem form", async () => (await markerForm(page)) === "problem");
    // ONE marker, not a ring beside a dot.
    const count = () => page.evaluate(() => document.querySelectorAll(".pair-pill-marker").length);
    if ((await count()) !== 1) throw new Error(`${await count()} markers on the pill: it has one`);

    // The folder recovers: the marker becomes the ring, and it is the dot that was swapped, not the pill —
    // the keyboard could be standing on it.
    await page.evaluate(() => {
      document.querySelector(".pair-pill").__probe = "the same pill";
    });
    bridge.states = {};
    await poll(page, bridge);
    await until("the ring", async () => (await markerForm(page)) === "decision");
    const ringName = await page.evaluate(() =>
      document.querySelector(".pair-pill").getAttribute("aria-label"),
    );
    if (ringName !== "Folder docs. Another folder has something waiting.") {
      throw new Error(`the ring's accessible name is ${JSON.stringify(ringName)}`);
    }
    bridge.states = { music: FAILED() };
    await poll(page, bridge);
    await until("the problem form again", async () => (await markerForm(page)) === "problem");
    if (!(await page.evaluate(() => document.querySelector(".pair-pill")?.__probe === "the same pill"))) {
      throw new Error("the pill was rebuilt when its marker changed form");
    }
    if ((await count()) !== 1) throw new Error("the marker changing form left two on the pill");
    await page.close();
  },
);

await scenario(
  "when the daemon stops answering, no folder in the list says it is up to date and the pill stops marking",
  async () => {
    // Two folders, one with something waiting on a person (the ring) — then every status read fails. The
    // store keeps the last roster and states, which used to read `up to date` beside a chip saying
    // `unreachable` (#246).
    const bridge = new Bridge({ docs: [], photos: [deletion("a.txt")] });
    const page = await open(bridge);
    await until("the ring", async () => (await markerForm(page)) === "decision");
    await openList(page);
    // The positive control: while it answers, the list says so and counts.
    const live = await listRows(page);
    if (live.docs[0] !== CHROME.pair.states.idle || live.photos[1] !== CHROME.chips.waiting(1)) {
      throw new Error(`the live list is ${JSON.stringify(live)}`);
    }

    bridge.down = true;
    await poll(page, bridge);
    await until("the chip to say so", async () => (await chipText(page)) === "unreachable");
    const stale = await listRows(page);
    for (const [name, [word, count]] of Object.entries(stale)) {
      if (word !== CHROME.pair.states.unreachable || count !== "") {
        throw new Error(`${name} reads ${JSON.stringify([word, count])} with the daemon stopped`);
      }
    }
    if (await markerForm(page)) throw new Error("the pill is still marked from the last answer");

    // And it comes back when the daemon does: the rows are the live ones again.
    bridge.down = false;
    await poll(page, bridge);
    await until("the live list", async () => (await listRows(page)).docs[0] === CHROME.pair.states.idle);
    await until("the ring again", async () => (await markerForm(page)) === "decision");
    await page.close();
  },
);

/** One tray-panel poll, forced and awaited (`pair-selected` is the page's "look again"). */
async function trayPoll(page, bridge) {
  const before = bridge.called("tray_status").length;
  await page.evaluate(() => window.__listeners["pair-selected"]({ payload: null }));
  await until("a tray poll", () => bridge.called("tray_status").length > before);
  await settle(page);
}

await scenario(
  "the tray panel over a daemon that stopped answering is the stopped-daemon panel, not the last folders'",
  async () => {
    // Review of #447: with two folders the panel drew from the roster the store keeps across a failed read, so a
    // stopped daemon read `Up to date`, `Sync now` and a pause row per folder (#246, in the one surface without
    // the window's guard).
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge, "?surface=tray");
    await until("the folders' panel", () => hasButton(page, TRAY.pausePair("photos")));
    // The positive control: while it answers, this is the several-folder panel, and says it is up to date.
    if (!(await pageText(page)).includes(MAIN.compact.upToDate)) {
      throw new Error(`the live panel does not say up to date: ${JSON.stringify(await pageText(page))}`);
    }

    bridge.down = true;
    await trayPoll(page, bridge);
    await until("the folder rows to go", async () => !(await hasButton(page, TRAY.pausePair("photos"))));
    const stopped = await pageText(page);
    if (stopped.includes(MAIN.compact.upToDate)) {
      throw new Error(`a stopped daemon is drawn as up to date: ${JSON.stringify(stopped)}`);
    }
    for (const label of [TRAY.pausePair("docs"), TRAY.pausePair("photos")]) {
      if (await hasButton(page, label)) throw new Error(`"${label}" is offered over a stopped daemon`);
    }

    // The panel a ONE-folder install draws over the same stopped daemon, which is the one it must be.
    const single = new Bridge({ docs: [] }, { names: ["docs"] });
    single.down = true;
    const reference = await open(single, "?surface=tray");
    await until("the first poll", () => single.called("tray_status").length >= 1);
    await settle(reference);
    const expected = await pageText(reference);
    if (stopped !== expected) {
      throw new Error(
        `the panel is ${JSON.stringify(stopped)}, a one-folder panel says ${JSON.stringify(expected)}`,
      );
    }
    await reference.close();

    // And it is the folders' panel again when the daemon answers.
    bridge.down = false;
    await trayPoll(page, bridge);
    await until("the folders' panel again", () => hasButton(page, TRAY.pausePair("photos")));
    await page.close();
  },
);

await scenario(
  "a failed read of a folder that is not on screen changes nothing about the folder that is",
  async () => {
    // `photos` has a deletion waiting, so the poll reads it by name (E6). That one request fails — the reply is
    // Rust's unreachable payload, stamped with the folder ON SCREEN. Filed under that, the chip said
    // `unreachable`, the list's rows said so too and the pill's ring went, until the next poll.
    const bridge = new Bridge({ docs: [], photos: [deletion("a.txt")] });
    const page = await open(bridge);
    await until("the ring", async () => (await markerForm(page)) === "decision");
    await openList(page);
    bridge.failFor.add("photos");
    const named = () => bridge.called("get_status").filter((call) => call.args?.pair === "photos").length;
    const before = named();
    await poll(page, bridge);
    await until("the read of photos", () => named() > before);
    await settle(page);
    await settle(page);
    if ((await chipText(page)) !== CHROME.chips.idle) {
      throw new Error(`the chip took another folder's failed read: ${JSON.stringify(await chipText(page))}`);
    }
    const rows = await listRows(page);
    if (rows.docs[0] !== CHROME.pair.states.idle) {
      throw new Error(
        `the folder on screen reads ${JSON.stringify(rows.docs)} after another folder's failed read`,
      );
    }
    if ((await markerForm(page)) !== "decision")
      throw new Error("the ring went with another folder's failed read");
    await page.close();
  },
);

await scenario(
  "a pause the daemon could not save is said under the hero and the next press retires it",
  async () => {
    // `trackPause`: the daemon APPLIED the pause (the hero now offers Resume), which is what makes a notice
    // about it true — a status that said the folder was not paused would retire it (see the scenarios below).
    const bridge = new Bridge({ docs: [], photos: [] }, { trackPause: true });
    const page = await open(bridge);
    await until("the hero", () => hasButton(page, TRAY.pausePair("docs")));
    bridge.unsaved = "disk full";
    await press(page, TRAY.pausePair("docs"));
    await until("the notice", async () => (await pageText(page)).includes(MAIN.notice.pauseUnsaved));
    const shown = await pageText(page);
    if (!shown.includes("disk full")) throw new Error("the daemon's own reason is not quoted");
    if (!shown.includes(MAIN.notice.pauseUnsavedSub)) throw new Error("the sentence is not there");

    bridge.unsaved = null;
    await press(page, TRAY.resumePair("docs"));
    await until("the notice to go", async () => !(await pageText(page)).includes(MAIN.notice.pauseUnsaved));

    // THE TRAY'S ROW: its reply cannot be shown in a panel that every row dismisses, so Rust tells the window.
    await page.evaluate(() =>
      window.__listeners["pause-unsaved"]({
        payload: { pair: "docs", paused: false, reason: "read-only index" },
      }),
    );
    await until("the resume notice", async () => (await pageText(page)).includes(MAIN.notice.resumeUnsaved));
    if (!(await pageText(page)).includes("read-only index"))
      throw new Error("the tray row's reason is not quoted");
    // …and a notice about a folder that is not the one on screen says nothing here.
    await select(page, bridge, "photos");
    if ((await pageText(page)).includes(MAIN.notice.resumeUnsaved)) {
      throw new Error("a notice about docs is on photos' screen");
    }
    await page.evaluate(() =>
      window.__listeners["pause-unsaved"]({ payload: { pair: "docs", paused: true, reason: "x" } }),
    );
    await settle(page);
    if ((await pageText(page)).includes(MAIN.notice.pauseUnsaved)) {
      throw new Error("an event for another folder drew a notice");
    }
    await page.close();
  },
);

await scenario("a folder the daemon does not run is named, with the one thing to do about it", async () => {
  const queues = { docs: [], music: [], photos: [] };
  const bridge = new Bridge(queues, {
    names: ["docs", "music"],
    roster: ["docs", "music", "photos"],
    pairUnknown: "photos",
  });
  const page = await open(bridge);
  await until("the notice", async () =>
    (await pageText(page)).includes(MAIN.notice.pairNotRunning("photos")),
  );
  // The window is on the default folder meanwhile, and says it is.
  await until("docs on screen", () => hasButton(page, TRAY.pausePair("docs")));

  await press(page, MAIN.notice.restartSyncing);
  await until("the restart", () => bridge.called("restart_service").length === 1);
  if (bridge.called("restart_service")[0].args?.onlyIfRunning !== false) {
    throw new Error("the restart was `only if running`: the daemon IS running, on the old settings");
  }
  await until(
    "the notice to go",
    async () => !(await pageText(page)).includes(MAIN.notice.pairNotRunning("photos")),
  );
  await page.close();

  // A restart that did not work says so, and quotes the daemon.
  const failing = new Bridge(queues, {
    names: ["docs", "music"],
    roster: ["docs", "music", "photos"],
    pairUnknown: "photos",
  });
  failing.restartEnding = "never_stopped";
  const failed = await open(failing);
  await press(failed, MAIN.notice.restartSyncing);
  await until("the failure", async () => (await pageText(failed)).includes(MAIN.notice.restartFailed));
  if (!(await pageText(failed)).includes("it would not stop")) throw new Error("the reason is not quoted");
  await failed.close();

  // A remembered folder the settings file does not list is stale preference: nothing to say, nothing to restart.
  const stale = new Bridge(
    { docs: [], music: [] },
    { names: ["docs", "music"], roster: ["docs", "music"], pairUnknown: "photos" },
  );
  const quiet = await open(stale);
  await until("the hero", () => hasButton(quiet, TRAY.pausePair("docs")));
  await settle(quiet);
  if ((await pageText(quiet)).includes("isn't being synced yet"))
    throw new Error("a folder the file does not list got a notice");
  await quiet.close();
});

// ---- the notices outlive a press and end when they stop being true (the review of #447) ---------------

/** Does the page say `text` right now? */
const says = async (page, text) => (await pageText(page)).includes(text);

await scenario(
  "a pause that was not saved stops being said when the folder is resumed elsewhere, and so does a resume",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { trackPause: true });
    const page = await open(bridge);
    await until("the hero", () => hasButton(page, TRAY.pausePair("docs")));
    bridge.unsaved = "disk full";
    await press(page, TRAY.pausePair("docs"));
    await until("the notice", () => says(page, MAIN.notice.pauseUnsaved));
    await until("Resume", () => hasButton(page, TRAY.resumePair("docs")));
    // The folder is still paused, so the sentence is still true: a poll that says so keeps it.
    await poll(page, bridge);
    await settle(page);
    if (!(await says(page, MAIN.notice.pauseUnsaved))) {
      throw new Error("the notice went while the folder it is about is still paused");
    }

    // A SAVED resume from the tray, or `proton-sync resume`: no event reaches the window, only the next
    // status — which says the folder is not paused, so "paused, but not saved" is no longer a thing to say.
    bridge.unsaved = null;
    bridge.pausedPairs.delete("docs");
    await poll(page, bridge);
    await until("the notice to go", async () => !(await says(page, MAIN.notice.pauseUnsaved)));
    // The positive control: the hero offers Pause again, so the folder really is running.
    await until("the hero to offer Pause", () => hasButton(page, TRAY.pausePair("docs")));

    // THE OTHER DIRECTION. Paused (saved) → resumed, not saved → paused again elsewhere, saved.
    bridge.pausedPairs.add("docs");
    await poll(page, bridge);
    await until("Resume", () => hasButton(page, TRAY.resumePair("docs")));
    bridge.unsaved = "read-only index";
    await press(page, TRAY.resumePair("docs"));
    await until("the resume notice", () => says(page, MAIN.notice.resumeUnsaved));
    await poll(page, bridge);
    await settle(page);
    if (!(await says(page, MAIN.notice.resumeUnsaved))) {
      throw new Error("the resume notice went while the folder is still running");
    }
    bridge.unsaved = null;
    bridge.pausedPairs.add("docs");
    await poll(page, bridge);
    await until("the resume notice to go", async () => !(await says(page, MAIN.notice.resumeUnsaved)));
    await page.close();
  },
);

await scenario(
  "a status reply that left before the pause cannot retire the notice the pause produced",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { trackPause: true });
    const page = await open(bridge);
    await until("the hero", () => hasButton(page, TRAY.pausePair("docs")));
    // A poll leaves NOW, composed from the state before the pause (docs running), and is answered after it.
    const releaseOld = bridge.holdLate("get_status");
    const before = bridge.called("get_status").length;
    await page.evaluate(() => window.__listeners["pair-selected"]({ payload: null }));
    await until("the poll to leave", () => bridge.called("get_status").length > before);

    bridge.unsaved = "disk full";
    await press(page, TRAY.pausePair("docs"));
    await until("the notice", () => says(page, MAIN.notice.pauseUnsaved));
    await settle(page); // the poll the press asks for lands: paused
    releaseOld(); // …and then the OLD reply: docs not paused, issued before the notice existed
    await settle(page);
    await settle(page);
    if (!(await says(page, MAIN.notice.pauseUnsaved))) {
      throw new Error("an answer to a request issued before the pause retired the notice about the pause");
    }
    await page.close();
  },
);

await scenario(
  "an unsaved pause for a folder that is not on screen is kept for when that folder is looked at",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { trackPause: true });
    const page = await open(bridge);
    await until("the hero", () => hasButton(page, TRAY.pausePair("docs")));
    // The tray paused photos and the daemon could not save it. The panel is dismissed by the time the reply
    // arrives, so Rust tells the window — which is showing docs.
    bridge.pausedPairs.add("photos");
    await page.evaluate(() =>
      window.__listeners["pause-unsaved"]({ payload: { pair: "photos", paused: true, reason: "disk full" } }),
    );
    await settle(page);
    if (await says(page, MAIN.notice.pauseUnsaved)) throw new Error("photos' notice is on docs' screen");

    await select(page, bridge, "photos");
    await until("photos' notice", () => says(page, MAIN.notice.pauseUnsaved));
    if (!(await says(page, "disk full"))) throw new Error("the daemon's reason is not quoted");

    // It belongs to photos: docs does not show it, and photos still has it when the window comes back.
    await select(page, bridge, "docs");
    if (await says(page, MAIN.notice.pauseUnsaved)) throw new Error("photos' notice is on docs' screen");
    await select(page, bridge, "photos");
    await until("photos' notice again", () => says(page, MAIN.notice.pauseUnsaved));

    // And it ends as every notice does: photos is resumed elsewhere.
    bridge.pausedPairs.delete("photos");
    await poll(page, bridge);
    await until("the notice to go", async () => !(await says(page, MAIN.notice.pauseUnsaved)));

    // An event for a folder the daemon does not list is about nothing this window can show, and is not kept
    // against the day a folder of that name appears.
    await page.evaluate(() =>
      window.__listeners["pause-unsaved"]({ payload: { pair: "ghost", paused: true, reason: "disk full" } }),
    );
    await settle(page);
    bridge.names = [...bridge.names, "ghost"]; // a new array: the default is the module's PAIRS, shared by every bridge
    bridge.queues.ghost = [];
    bridge.pausedPairs.add("ghost"); // …and it IS paused, which is what would make a kept notice true
    await poll(page, bridge);
    await select(page, bridge, "ghost");
    await until("the window on ghost", () => hasButton(page, TRAY.resumePair("ghost")));
    if (await says(page, MAIN.notice.pauseUnsaved)) {
      throw new Error("a notice kept for a folder the daemon did not list is on its screen now that it does");
    }
    await page.close();
  },
);

await scenario(
  "the notice for a daemon that stops answering is not kept for the one that comes back",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { trackPause: true });
    const page = await open(bridge);
    await until("the hero", () => hasButton(page, TRAY.pausePair("docs")));
    bridge.unsaved = "disk full";
    await press(page, TRAY.pausePair("docs"));
    await until("the notice", () => says(page, MAIN.notice.pauseUnsaved));

    // "If syncing restarts first" is about a restart; a daemon that stopped answering has been through one.
    bridge.down = true;
    await poll(page, bridge);
    await until("the stopped daemon", async () => (await chipText(page)) === "unreachable");
    if (await says(page, MAIN.notice.pauseUnsaved)) {
      throw new Error("the notice about an unsaved pause is still on screen with the daemon stopped");
    }
    // It does not come back with the daemon: the pause is in the daemon's index or it is not, and the
    // status says which. (The folder is still paused in this bridge, which is what would revive it.)
    bridge.down = false;
    await poll(page, bridge);
    await until("the paused hero", () => hasButton(page, TRAY.resumePair("docs")));
    await settle(page);
    if (await says(page, MAIN.notice.pauseUnsaved)) {
      throw new Error("the notice came back when the daemon did");
    }
    await page.close();
  },
);

await scenario(
  "a failed restart is remembered for the folder it was for, not for the next one the daemon does not run",
  async () => {
    const bridge = new Bridge(
      { docs: [], music: [] },
      {
        names: ["docs", "music"],
        roster: ["docs", "music", "photos", "videos"],
        pairUnknown: "photos",
      },
    );
    bridge.restartEnding = "never_stopped";
    const page = await open(bridge);
    await until("photos' notice", () => says(page, MAIN.notice.pairNotRunning("photos")));
    await press(page, MAIN.notice.restartSyncing);
    await until("the failure", () => says(page, MAIN.notice.restartFailed));
    if (!(await says(page, "it would not stop"))) throw new Error("the daemon's reason is not quoted");

    // The window remembers another folder the daemon does not run. Its notice is its own: it has not been
    // tried, so it does not open already saying a restart failed, or quote the reason for another folder's.
    bridge.pairUnknown = "videos";
    await poll(page, bridge);
    await until("videos' notice", () => says(page, MAIN.notice.pairNotRunning("videos")));
    const shown = await pageText(page);
    if (shown.includes(MAIN.notice.restartFailed) || shown.includes("it would not stop")) {
      throw new Error(`videos' notice quotes photos' failed restart: ${JSON.stringify(shown)}`);
    }
    if (!shown.includes(MAIN.notice.pairNotRunningSub)) throw new Error("videos' own sentence is missing");

    // And photos' failure is not waiting to come back: the notice for it starts over.
    bridge.pairUnknown = "photos";
    await poll(page, bridge);
    await until("photos' notice again", () => says(page, MAIN.notice.pairNotRunning("photos")));
    if (await says(page, MAIN.notice.restartFailed)) {
      throw new Error("photos' notice opens with the failure of a restart attempted before");
    }
    await page.close();
  },
);

// ---- 25-35. the Settings screen's handlers are the screen's, and the conflict tally is the visit's -------

/** A conflict card's choice button: its text is the name AND the consequence, so it is found by its name. */
async function pressChoice(page, label) {
  const handle = await until("the card's choices", async () => {
    const found = await page.evaluateHandle(
      (text) =>
        [...document.querySelectorAll("button")].find(
          (b) => b.querySelector(".btn-choice-name")?.textContent.trim() === text,
        ) ?? null,
      label,
    );
    return found.asElement();
  });
  await handle.evaluate((button) => button.click());
}

/** A handle on the first button reading exactly `label`, or null. */
const buttonNamed = (label) => async (page) =>
  (
    await page.evaluateHandle(
      (text) => [...document.querySelectorAll("button")].find((b) => b.textContent.trim() === text) ?? null,
      label,
    )
  ).asElement();
const fieldNamed = (selector) => async (page) =>
  (await page.evaluateHandle((sel) => document.querySelector(sel), selector)).asElement();

const typeInto = (value) => (handle) =>
  handle.evaluate((input, text) => {
    input.value = text;
    input.dispatchEvent(new Event("input", { bubbles: true }));
  }, value);
const clickIt = (handle) => handle.evaluate((node) => node.click());

const SETTINGS_FILES = {
  docs: {
    exclude: ["*.tmp", "*.log"],
    events_driven: true,
    remote_root: "/Drive/docs",
    deletion_policy: "ask_every_time",
  },
  photos: {
    exclude: ["*.raw"],
    events_driven: true,
    remote_root: "/Drive/photos",
    deletion_policy: "only_permanent",
  },
};

/**
 * Open Settings on `tab` for docs and take hold of the control the person is about to use; MOVE THE
 * SELECTION under it; then use it — the press (or the keystroke) arrives after the switch, which is what a
 * click landing in the gap looks like. Back on docs, `Save` must write what was staged, to docs: the
 * handler is bound to the folder the screen was DRAWN for, not to the one selected when it runs.
 */
async function stagedInTheGap({ tab, control, use, key, expected }) {
  const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_FILES });
  const page = await open(bridge);
  await press(page, "Settings");
  if (tab) await press(page, tab);
  const handle = await until("the control", () => control(page));
  await select(page, bridge, "photos");
  await use(handle);
  await settle(page);
  await select(page, bridge, "docs");
  await press(page, SETTINGS.save);
  await until("docs' write", () => bridge.called("write_config").length >= 1);
  expectPair(bridge.called("write_config"), "docs", "write_config");
  const update = bridge.called("write_config")[0].args.update;
  if (JSON.stringify(update[key]) !== JSON.stringify(expected)) {
    throw new Error(`the write carried ${JSON.stringify(update)}, not ${key} = ${JSON.stringify(expected)}`);
  }
  if (bridge.called("write_config").length !== 1) throw new Error("more than one write went out");
  await page.close();
}

await scenario("a typed remote folder is staged for the folder the screen was drawn for (onRoot)", () =>
  stagedInTheGap({
    control: fieldNamed('[data-field="remote_root"]'),
    use: typeInto("/Drive/typed"),
    key: "remote_root",
    expected: "/Drive/typed",
  }),
);

await scenario("a typed Advanced field is staged for the folder the screen was drawn for (onField)", () =>
  stagedInTheGap({
    tab: SETTINGS.tabs.advanced,
    control: fieldNamed('[data-sfocus="field:proton_cli"]'),
    use: typeInto("/opt/proton-drive"),
    key: "proton_cli",
    expected: "/opt/proton-drive",
  }),
);

await scenario("the live-updates switch is staged for the folder the screen was drawn for (onEvents)", () =>
  stagedInTheGap({
    control: fieldNamed('button[role="switch"]'),
    use: clickIt,
    key: "events_driven",
    expected: false,
  }),
);

await scenario("a rule removed in the gap leaves the folder the Remove was drawn for (onRemoveRule)", () =>
  stagedInTheGap({
    tab: SETTINGS.tabs.skip,
    control: buttonNamed(SETTINGS.remove),
    use: clickIt,
    key: "exclude",
    expected: ["*.log"],
  }),
);

await scenario("a rule added in the gap goes to the folder the Add was drawn for (onAddRule)", async () => {
  const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_FILES });
  const page = await open(bridge);
  await press(page, "Settings");
  await press(page, SETTINGS.tabs.skip);
  await until("the draft field", () => page.$('[data-sfocus="field:draft-exclude"]'));
  await page.type('[data-sfocus="field:draft-exclude"]', "*.psd"); // the draft is docs'
  await settle(page);
  const add = await until("the Add button", () => buttonNamed(SETTINGS.add)(page));
  await select(page, bridge, "photos");
  await clickIt(add);
  await settle(page);
  await select(page, bridge, "docs");
  await press(page, SETTINGS.save);
  await until("docs' write", () => bridge.called("write_config").length >= 1);
  expectPair(bridge.called("write_config"), "docs", "write_config");
  const sent = bridge.called("write_config")[0].args.update.exclude;
  if (JSON.stringify(sent) !== JSON.stringify(["*.tmp", "*.log", "*.psd"])) {
    throw new Error(`the write carried exclude = ${JSON.stringify(sent)}`);
  }
  await page.close();
});

await scenario(
  "a folder chosen while the picker was open is staged for the folder it was opened for (chooseLocalRoot)",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_FILES });
    const page = await open(bridge);
    await press(page, "Settings");
    const release = bridge.hold("choose_folder");
    await press(page, SETTINGS.choose); // the picker opens on docs' root, and is modal
    await until("the picker", () => bridge.called("choose_folder").length === 1);
    await select(page, bridge, "photos"); // the selection moves while it is open
    release(); // …and the person picks
    await settle(page);
    await settle(page);
    await select(page, bridge, "docs");
    await press(page, SETTINGS.save);
    await until("docs' write", () => bridge.called("write_config").length >= 1);
    expectPair(bridge.called("write_config"), "docs", "write_config");
    if (bridge.called("write_config")[0].args.update.local_root !== "/picked/folder") {
      throw new Error(`the write carried ${JSON.stringify(bridge.called("write_config")[0].args.update)}`);
    }
    await page.close();
  },
);

await scenario("Restart it now acts for the folder whose bar it was drawn on (onRestart)", async () => {
  const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_FILES });
  const page = await open(bridge);
  await openDeletions(page);
  // A save whose restart did not finish leaves the retry on the bar…
  bridge.restartEnding = "never_stopped";
  await pressCard(page, SETTINGS.askNever);
  await press(page, SETTINGS.save);
  await until("the retry", () => hasButton(page, SETTINGS.restart));
  // …and a failed sweep leaves a notice on docs' bar, which a restart that DOES finish retires.
  bridge.resyncError = "boom";
  await press(page, SETTINGS.tabs.folders);
  await press(page, SETTINGS.sweepNow);
  await until("docs' notice", async () => (await pageText(page)).includes("boom"));
  const retry = await until("the retry button", () => buttonNamed(SETTINGS.restart)(page));
  bridge.restartEnding = "restarted";
  await select(page, bridge, "photos");
  await clickIt(retry); // drawn for docs, pressed while photos is on screen
  await until("the restart", () => bridge.called("restart_service").length >= 2);
  await settle(page);
  await select(page, bridge, "docs");
  if ((await pageText(page)).includes("boom")) {
    throw new Error("the restart cleared photos' notice and left docs' — it acted for the folder on screen");
  }
  await page.close();
});

await scenario(
  "a folder whose settings have not arrived draws none, not another folder's or an empty file's",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS });
    const page = await open(bridge);
    await openDeletions(page); // docs' policy is drawn
    const releasePhotos = bridge.hold("read_config", "photos");
    await select(page, bridge, "photos"); // its settings are asked for and held
    await until("the read for photos", () =>
      bridge.called("read_config").some((c) => c.args?.pair === "photos"),
    );
    await settle(page);
    const drawn = await policyCards(page);
    if (Object.values(drawn).some(Boolean)) {
      throw new Error(
        `a policy card is chosen for photos before its settings arrived: ${JSON.stringify(drawn)}`,
      );
    }
    releasePhotos();
    await until("photos' own policy", async () => (await policyCards(page))[SETTINGS.askPermanent]);
    await page.close();
  },
);

await scenario("Discard changes discards the folder on screen's edits and no other folder's", async () => {
  const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS });
  const page = await open(bridge);
  await openDeletions(page);
  await pressCard(page, SETTINGS.askNever); // staged for docs
  await select(page, bridge, "photos");
  await until("photos' own policy", async () => (await policyCards(page))[SETTINGS.askPermanent]);
  await pressCard(page, SETTINGS.askEvery); // staged for photos
  await press(page, SETTINGS.discard); // photos' only
  await until("photos back to its own policy", async () => (await policyCards(page))[SETTINGS.askPermanent]);
  await select(page, bridge, "docs");
  const docs = await until("docs' staged policy", async () => {
    const cards = await policyCards(page);
    return cards[SETTINGS.askNever] ? cards : null;
  });
  if (!docs[SETTINGS.askNever]) throw new Error("photos' Discard threw docs' staged edit away");
  await page.close();
});

await scenario(
  "a keystroke typed while a save is in flight is still staged when the save lands",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS });
    const page = await open(bridge);
    await openDeletions(page);
    await pressCard(page, SETTINGS.askNever); // the edit that is saved
    const release = bridge.hold("write_config");
    await press(page, SETTINGS.save);
    await until("the write to leave", () => bridge.called("write_config").length === 1);
    // While it is out, a second edit is staged: a different setting, so the card it chooses is its own.
    await pressCard(page, SETTINGS.disposalPermanent);
    release();
    await until("the save to finish", async () => !(await pageText(page)).includes(SETTINGS.saving));
    await settle(page);
    const armed = await page.evaluate(
      (label) =>
        [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === label && !b.disabled),
      SETTINGS.save,
    );
    if (!armed) throw new Error("the edit typed during the save was discarded with the one that was saved");
    await page.close();
  },
);

for (const awkward of ["constructor", "__proto__"]) {
  await scenario(
    `a folder named ${awkward} is chosen, shown, staged for and saved like any other (names are not object keys)`,
    async () => {
      // A name the engine accepts that a plain object answers from its prototype: every table the window
      // keys by folder must be a Map, or the screen below throws or draws another folder's settings.
      const queues = { docs: [] };
      Object.defineProperty(queues, awkward, {
        value: [],
        enumerable: true,
        configurable: true,
        writable: true,
      });
      const bridge = new Bridge(queues, {
        names: ["docs", awkward],
        configs: { docs: { remote_root: "/Drive/docs" } },
      });
      const page = await open(bridge);
      await until("the pill", () => page.$(".pair-pill"));
      await page.focus(".pair-pill");
      await page.keyboard.press("ArrowDown");
      await until("the list", () => page.$(".pair-row"));
      await page.evaluate((name) => {
        [...document.querySelectorAll(".pair-row")].find((row) => row.dataset.pair === name).click();
      }, awkward);
      await until("select_pair", () => bridge.called("select_pair").length === 1);
      await until("the window on it", () => hasButton(page, TRAY.pausePair(awkward)));
      await press(page, "Settings");
      const field = await until("the remote field", () => fieldNamed('[data-field="remote_root"]')(page));
      await typeInto("/Drive/typed")(field);
      await settle(page);
      await press(page, SETTINGS.save);
      await until("the write", () => bridge.called("write_config").length >= 1);
      expectPair(bridge.called("write_config"), awkward, "write_config");
      if (bridge.called("write_config")[0].args.update.remote_root !== "/Drive/typed") {
        throw new Error(`the write carried ${JSON.stringify(bridge.called("write_config")[0].args.update)}`);
      }
      await page.close();
    },
  );
}

await scenario(
  "the tally of a late decision counts the visit it was made on, not the one now showing",
  async () => {
    // docs and photos each have one conflict. The decision on docs' is held; the selection moves to photos,
    // whose screen starts a visit of its own; the decision lands. Photos is then settled by hand. The
    // cleared screen says how many YOU settled here — one — and not two, which is what counting docs' late
    // decision into photos' tally says.
    const bridge = new Bridge(
      { docs: [], photos: [] },
      { conflicts: { docs: [CONFLICT], photos: [CONFLICT] } },
    );
    const release = bridge.hold("resolve_conflict", "docs");
    const page = await open(bridge);
    await press(page, MAIN.band.conflictAction);
    await pressChoice(page, CONFLICTS.keepBoth);
    await until("docs' decision to leave", () => bridge.called("resolve_conflict").length === 1);
    await select(page, bridge, "photos");
    release();
    await until(
      "the rescan of docs",
      () => bridge.called("scan_conflicts").filter((c) => c.args?.pair === "docs").length >= 2,
    );
    await settle(page);
    await pressChoice(page, CONFLICTS.keepBoth); // photos' own conflict
    await until("the cleared screen", async () => (await pageText(page)).includes(CONFLICTS.clearedTitle));
    const shown = await pageText(page);
    const once = CONFLICTS.clearedSub({ total: 1, keptBoth: 1, tookProton: 0 });
    if (!shown.includes(once)) {
      throw new Error(`the cleared screen says ${JSON.stringify(shown)}, not ${JSON.stringify(once)}`);
    }
    await page.close();
  },
);

// ---- the settings handlers the first pass of the gate left unpinned (the review of #447) --------------------

/** A policy or disposal card on the Deletions tab, by its title. */
const cardNamed = (title) => async (page) =>
  (
    await page.evaluateHandle(
      (label) =>
        [...document.querySelectorAll(".settings-cards .radio-card")].find(
          (card) => card.querySelector(".radio-title")?.textContent.trim() === label,
        ) ?? null,
      title,
    )
  ).asElement();

await scenario(
  "a deletion policy chosen in the gap is staged for the folder the card was drawn for (onPolicy)",
  () =>
    stagedInTheGap({
      tab: SETTINGS.tabs.deletions,
      control: cardNamed(SETTINGS.askNever),
      use: clickIt,
      key: "deletion_policy",
      expected: "never",
    }),
);

await scenario(
  "a disposal chosen in the gap is staged for the folder the card was drawn for (onDisposal)",
  () =>
    stagedInTheGap({
      tab: SETTINGS.tabs.deletions,
      control: cardNamed(SETTINGS.disposalPermanent),
      use: clickIt,
      key: "local_delete_mode",
      expected: "permanent",
    }),
);

await scenario(
  "a sweep day chosen in the gap is staged for the folder the chip was drawn for (onSchedule)",
  () =>
    stagedInTheGap({
      control: buttonNamed("Wed"),
      use: clickIt,
      key: "full_scan_schedule",
      expected: "weekly wed 03:00",
    }),
);

await scenario(
  "a draft typed in the gap is kept for the folder the field was drawn for (onDraft)",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_FILES });
    const page = await open(bridge);
    await press(page, "Settings");
    await press(page, SETTINGS.tabs.skip);
    const field = await until("the draft field", () =>
      fieldNamed('[data-sfocus="field:draft-exclude"]')(page),
    );
    const draftHere = () =>
      page.evaluate(() => document.querySelector('[data-sfocus="field:draft-exclude"]')?.value ?? null);
    await select(page, bridge, "photos");
    await typeInto("*.psd")(field); // typed after the selection moved: the field is docs'
    await settle(page);
    if ((await draftHere()) !== "")
      throw new Error(`photos' field reads ${JSON.stringify(await draftHere())}`);
    await select(page, bridge, "docs");
    if ((await draftHere()) !== "*.psd") {
      throw new Error(`docs' draft is ${JSON.stringify(await draftHere())}, not what was typed for it`);
    }
    await page.close();
  },
);

await scenario(
  "an include added in the gap goes to the folder the Add was drawn for (onAddInclude)",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_FILES });
    const page = await open(bridge);
    await press(page, "Settings");
    await press(page, SETTINGS.tabs.advanced);
    await until("the include field", () => page.$('[data-sfocus="field:draft-include"]'));
    await page.type('[data-sfocus="field:draft-include"]', "*.md"); // the draft is docs'
    await settle(page);
    const add = await until("the include's Add", async () =>
      (
        await page.evaluateHandle(
          () =>
            document
              .querySelector('[data-sfocus="field:draft-include"]')
              ?.parentElement?.querySelector("button") ?? null,
        )
      ).asElement(),
    );
    await select(page, bridge, "photos");
    await clickIt(add);
    await settle(page);
    await select(page, bridge, "docs");
    await press(page, SETTINGS.save);
    await until("docs' write", () => bridge.called("write_config").length >= 1);
    expectPair(bridge.called("write_config"), "docs", "write_config");
    const sent = bridge.called("write_config")[0].args.update.include;
    if (JSON.stringify(sent) !== JSON.stringify(["*.md"])) {
      throw new Error(`the write carried include = ${JSON.stringify(sent)}`);
    }
    await page.close();
  },
);

await scenario(
  "a full sweep asked for in the gap is for the folder the button was drawn for (onSweep)",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_FILES });
    const page = await open(bridge);
    await press(page, "Settings");
    const sweep = await until("the sweep button", () => buttonNamed(SETTINGS.sweepNow)(page));
    await select(page, bridge, "photos");
    await clickIt(sweep);
    await until("the sweep", () => bridge.called("resync").length === 1);
    expectPair(bridge.called("resync"), "docs", "resync");
    await page.close();
  },
);

// ---- what a switch drops, held on the real page and not only by the ledger's source scan -----------------

await scenario(
  "a switch drops the plan made for the folder that was left, and rehearses the one now shown",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await press(page, "Plan a sync");
    await until("docs' plan", () => says(page, "docs/gone.txt"));
    // Photos' rehearsal is held out, so what is on screen meanwhile is what the switch left behind.
    const release = bridge.hold("run_dry_run", "photos");
    await select(page, bridge, "photos");
    await until("photos' rehearsal to start", () =>
      bridge.called("run_dry_run").some((call) => call.args?.pair === "photos"),
    );
    if (await says(page, "docs/gone.txt")) {
      throw new Error("docs' plan is still on screen after the switch to photos");
    }
    release();
    await until("photos' plan", () => says(page, "photos/gone.txt"));
    if (await says(page, "docs/gone.txt")) throw new Error("docs' plan came back");
    await page.close();
  },
);

// ---- the folder list (the review of #447) ------------------------------------------------------------------

await scenario(
  "only the chosen folder is in the list's tab order, and the stop moves with the choice",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [], music: [] }, { names: ["docs", "photos", "music"] });
    const page = await open(bridge);
    await until("the pill", () => page.$(".pair-pill"));
    const stops = () =>
      page.evaluate(() =>
        Object.fromEntries(
          [...document.querySelectorAll(".pair-row")].map((row) => [row.dataset.pair, row.tabIndex]),
        ),
      );
    await openList(page);
    const first = await stops();
    if (JSON.stringify(first) !== JSON.stringify({ docs: 0, photos: -1, music: -1 })) {
      throw new Error(`the tab stops are ${JSON.stringify(first)}, not the chosen folder alone`);
    }
    // Choose photos: the stop is photos' once the window follows.
    await page.keyboard.press("ArrowDown");
    await page.keyboard.press("Enter");
    await until("select_pair", () => bridge.called("select_pair").length === 1);
    await settle(page);
    await until("the window on photos", () => hasButton(page, TRAY.pausePair("photos")));
    await openList(page);
    const after = await stops();
    if (JSON.stringify(after) !== JSON.stringify({ docs: -1, photos: 0, music: -1 })) {
      throw new Error(`after choosing photos the tab stops are ${JSON.stringify(after)}`);
    }
    await page.close();
  },
);

await scenario(
  "the folder list closes when the keyboard leaves it and stays while it moves inside",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await until("the pill", () => page.$(".pair-pill"));
    const listOpen = () => page.$(".pair-popover");
    await openList(page);
    await page.keyboard.press("ArrowDown"); // within the list
    await page.keyboard.press("ArrowUp");
    if (!(await listOpen())) throw new Error("the list closed while the keyboard moved inside it");
    await page.keyboard.down("Shift"); // back to the pill: still inside the control
    await page.keyboard.press("Tab");
    await page.keyboard.up("Shift");
    const onPill = await page.evaluate(() => document.activeElement?.classList.contains("pair-pill"));
    if (!onPill) throw new Error("Shift+Tab from the chosen row did not reach the pill");
    if (!(await listOpen())) throw new Error("the list closed when the keyboard went back to the pill");

    // Away: Tab from the chosen row leaves the control for the next thing on the page.
    await page.keyboard.press("ArrowDown"); // pill → the chosen row
    await until("the keyboard in the list", () =>
      page.evaluate(() => document.activeElement?.classList.contains("pair-row")),
    );
    await page.keyboard.press("Tab");
    await until("the list to close when the keyboard leaves", async () => !(await listOpen()));

    // …and so does focus landing anywhere else at all.
    await openList(page);
    await page.focus(".menu-btn");
    await until("the list to close on focus elsewhere", async () => !(await listOpen()));
    await page.close();
  },
);

await scenario("a long list of folders scrolls inside the window and every row can be reached", async () => {
  const names = Array.from({ length: 25 }, (_, i) => `folder${String(i + 1).padStart(2, "0")}`);
  const bridge = new Bridge(Object.fromEntries(names.map((name) => [name, []])), {
    names,
    selected: names[0],
  });
  const page = await open(bridge);
  await until("the pill", () => page.$(".pair-pill"));
  await openList(page);
  const geometry = () =>
    page.evaluate(() => {
      const list = document.querySelector(".pair-popover");
      const box = list.getBoundingClientRect();
      const focused = document.activeElement?.closest?.(".pair-row")?.getBoundingClientRect() ?? null;
      return {
        rows: list.querySelectorAll(".pair-row").length,
        top: box.top,
        bottom: box.bottom,
        scrolls: list.scrollHeight > list.clientHeight + 1,
        window: window.innerHeight,
        focused: focused ? { top: focused.top, bottom: focused.bottom } : null,
      };
    });
  const first = await geometry();
  if (first.rows !== 25) throw new Error(`${first.rows} rows in the list, expected 25`);
  // NOTHING PAINTS OUTSIDE THE WINDOW. Unbounded, 25 rows end near 830px in a 764px window.
  if (first.bottom > first.window) {
    throw new Error(`the list ends at ${first.bottom}px in a ${first.window}px window`);
  }
  if (!first.scrolls) throw new Error("25 folders do not scroll: the list is unbounded");

  // The last row is reachable, and the keyboard's row is visible INSIDE the list when it gets there.
  await page.keyboard.press("End");
  const last = await geometry();
  if (!last.focused || last.focused.bottom > last.bottom + 1 || last.focused.top < last.top - 1) {
    throw new Error(`End focused a row that is not visible in the list: ${JSON.stringify(last)}`);
  }
  const name = await page.evaluate(() => document.activeElement?.dataset?.pair);
  if (name !== names[24]) throw new Error(`End reached ${name}, not ${names[24]}`);
  await page.keyboard.press("Home");
  const home = await geometry();
  if (!home.focused || home.focused.top < home.top - 1 || home.focused.bottom > home.bottom + 1) {
    throw new Error(`Home focused a row that is not visible in the list: ${JSON.stringify(home)}`);
  }
  // A MOUSE REACHES THEM TOO. The wheel over the list scrolls it; a list that only focus can move (hidden
  // overflow scrolls programmatically and not by hand) leaves the last rows to the keyboard alone.
  const centre = await page.evaluate(() => {
    const box = document.querySelector(".pair-popover").getBoundingClientRect();
    return { x: box.left + box.width / 2, y: box.top + box.height / 2 };
  });
  await page.mouse.move(centre.x, centre.y);
  await page.mouse.wheel({ deltaY: 400 });
  await until("the wheel to scroll the list", () =>
    page.evaluate(() => document.querySelector(".pair-popover").scrollTop > 0),
  );
  // And the row in the middle of a walk stays visible too.
  for (let i = 0; i < 14; i += 1) await page.keyboard.press("ArrowDown");
  const middle = await geometry();
  if (!middle.focused || middle.focused.bottom > middle.bottom + 1 || middle.focused.top < middle.top - 1) {
    throw new Error(`ArrowDown walked to a row that is not visible in the list: ${JSON.stringify(middle)}`);
  }
  await page.close();
});

await scenario(
  "the list scrolls from the first folder that does not fit in 360px, and not before",
  async () => {
    // MEASURED, not computed: a row is 31px and the list has its own padding, so 11 rows are 355px and fit
    // under the 360px cap, while 12 are 386px and do not. The class used to be set from 11 folders (a list
    // of ten and a half rows, which the numbers do not support), so a list of 11 got a scroll container that
    // had nothing to scroll.
    for (const [count, scrolls] of [
      [2, false],
      [11, false],
      [12, true],
    ]) {
      const names = Array.from({ length: count }, (_, i) => `folder${String(i + 1).padStart(2, "0")}`);
      const bridge = new Bridge(Object.fromEntries(names.map((name) => [name, []])), {
        names,
        selected: names[0],
      });
      const page = await open(bridge);
      await until("the pill", () => page.$(".pair-pill"));
      await openList(page);
      const measured = await page.evaluate(() => {
        const list = document.querySelector(".pair-popover");
        return {
          rows: list.querySelectorAll(".pair-row").length,
          rowHeight: list.querySelector(".pair-row").getBoundingClientRect().height,
          height: list.getBoundingClientRect().height,
          overflows: list.scrollHeight > list.clientHeight + 1,
          class: list.classList.contains("is-scrolling"),
        };
      });
      console.log(`fidelity:pairs — ${count} folders measure ${JSON.stringify(measured)}`);
      if (measured.rows !== count) throw new Error(`${measured.rows} rows for ${count} folders`);
      if (measured.class !== scrolls || measured.overflows !== scrolls) {
        throw new Error(
          `${count} folders: class ${measured.class}, overflow ${measured.overflows}, expected both ${scrolls} ` +
            `(${JSON.stringify(measured)})`,
        );
      }
      if (scrolls && Math.round(measured.height) !== 360) {
        throw new Error(`the scrolling list is ${measured.height}px, not the 360px cap`);
      }
      await page.close();
    }
  },
);

// ---- the poll does not wait on the folders that are not on screen (the review of #447) ---------------------

await scenario(
  "a scan of another folder that never returns does not stop the shown folder's polls",
  async () => {
    // `photos` has a queue (so its status is fetched too) and a conflict scan that hangs — a network mount
    // that does not answer. Awaited before the next poll was scheduled, that scan stopped polling altogether.
    const bridge = new Bridge({ docs: [], photos: [deletion("a.txt")] });
    const release = bridge.hold("scan_conflicts", "photos");
    const page = await open(bridge);
    await page.bringToFront();
    await until("the scan to leave", () =>
      bridge.called("scan_conflicts").some((call) => call.args?.pair === "photos"),
    );
    const own = () => bridge.called("get_status").filter((call) => call.args?.pair == null).length;
    const before = own();
    await until("two more polls of the shown folder while that scan hangs", () => own() >= before + 2, 14000);

    // A hung scan is ONE scan, and its folder's status is read once: later polls do not stack more on it.
    const scans = bridge.called("scan_conflicts").filter((call) => call.args?.pair === "photos").length;
    const reads = bridge.called("get_status").filter((call) => call.args?.pair === "photos").length;
    if (scans !== 1 || reads > 1) {
      throw new Error(`while photos' scan hung it was asked ${scans} scan(s) and ${reads} status read(s)`);
    }
    release();
    await page.close();
  },
);

// ---- notifications per folder (#102 phase 5e) --------------------------------------------------------------
//
// The notifier is pure and `notifier-folders.test.js` drives it against the real store; what only the real page
// can show is the wiring: that the poll feeds it every folder, that the banner goes out naming one, and that a
// press on the banner's button acts on the folder the banner named and not on the one the window shows. The
// bridge answers `send_notification` with nothing and records the payload, so a "banner" here is a recorded
// call and no desktop is touched.

const nowSecs = () => Math.floor(Date.now() / 1000);
/** A withheld deletion that cannot be undone (`direction: local`, no trash) — the kind a banner is raised for. */
const permanent = (path) => ({ ...deletion(path), direction: "local" });
const bannersOf = (bridge) => bridge.called("send_notification").map((call) => call.args.payload);
const clickBanner = (page, payload) =>
  page.evaluate((body) => window.__listeners["notification-action"]({ payload: body }), payload);

/** A window on `queues` with banners switched on and every folder freshly synced. */
async function openWithBanners(queues, extra = {}) {
  const bridge = new Bridge(queues, { notifyPolicy: "only_when_needed", lastSync: nowSecs() - 60, ...extra });
  const page = await open(bridge);
  await page.bringToFront();
  return { bridge, page };
}

/** Poll until `count` banners have been sent. The first poll learns of another folder's queue, the next reads it. */
async function pollForBanners(page, bridge, count) {
  for (let i = 0; i < 6 && bannersOf(bridge).length < count; i += 1) await poll(page, bridge);
  if (bannersOf(bridge).length < count) {
    throw new Error(`${bannersOf(bridge).length} banner(s) after six polls, expected ${count}`);
  }
}

await scenario(
  "a banner about another folder names it, and `Keep them` keeps that folder's permanent deletions and no other's",
  async () => {
    const { bridge, page } = await openWithBanners({ docs: [], photos: [permanent("p.txt")] });
    await pollForBanners(page, bridge, 1);
    const [banner] = bannersOf(bridge);
    if (banner.kind !== "deletion" || banner.pair !== "photos" || banner.app !== "Drive Sync · photos") {
      throw new Error(`the banner was ${JSON.stringify(banner)}, expected a deletion about photos`);
    }
    // The window shows `docs`, which now has a permanent deletion of its own. The banner is about `photos`.
    bridge.queues.docs = [permanent("d.txt")];
    await poll(page, bridge);
    await clickBanner(page, { id: 1, kind: "deletion", action: "keep", pair: "photos" });
    await until("the keep", () => bridge.called("keep").length >= 1);
    await settle(page);
    expectPair(bridge.called("keep"), "photos", "keep from photos' banner");
    const targets = bridge.called("keep").map((call) => call.args.target);
    if (targets.length !== 1 || targets[0] !== "p.txt") {
      throw new Error(`the banner kept ${JSON.stringify(targets)}, expected photos' p.txt alone`);
    }
    await page.close();
  },
);

await scenario(
  "`Review` on another folder's banner selects that folder before it opens the Deletions screen",
  async () => {
    const { bridge, page } = await openWithBanners({
      docs: [permanent("d.txt")],
      photos: [permanent("p.txt")],
    });
    // Both folders have a queue; `docs` is first and on screen, so its banner is the one raised. Take the
    // click for `photos`' as a banner raised earlier would deliver it.
    await pollForBanners(page, bridge, 1);
    const release = bridge.hold("select_pair");
    await clickBanner(page, { id: 1, kind: "deletion", action: "review", pair: "photos" });
    await until("the selection to be asked for", () => bridge.called("select_pair").length === 1);
    await settle(page);
    // Not yet answered: the window must not have gone anywhere. A navigation here would draw the folder that
    // is still selected — `docs`' queue — under a banner that said `photos`.
    let text = await pageText(page);
    if (text.includes("d.txt") || text.includes("p.txt")) {
      throw new Error("the Deletions screen was drawn before the folder was selected");
    }
    release();
    await until("photos' queue on the Deletions screen", async () =>
      (await pageText(page)).includes("p.txt"),
    );
    text = await pageText(page);
    if (text.includes("d.txt")) throw new Error("the Deletions screen shows docs' queue as well");
    if (bridge.called("select_pair")[0].args?.name !== "photos") {
      throw new Error(`select_pair asked for ${JSON.stringify(bridge.called("select_pair")[0].args)}`);
    }
    await page.close();
  },
);

await scenario("`Try again now` on another folder's banner syncs that folder and no other", async () => {
  const bridge = new Bridge(
    { docs: [], photos: [] },
    { notifyPolicy: "only_when_needed", lastSync: nowSecs() - 60 },
  );
  bridge.lastSyncOf.photos = nowSecs() - 90_000; // more than a day: photos has not synced since yesterday
  const page = await open(bridge);
  await page.bringToFront();
  await pollForBanners(page, bridge, 1);
  const [banner] = bannersOf(bridge);
  if (banner.kind !== "outage" || banner.pair !== "photos") {
    throw new Error(`the banner was ${JSON.stringify(banner)}, expected an outage about photos`);
  }
  await clickBanner(page, { id: 1, kind: "outage", action: "retry", pair: "photos" });
  await until("the sync", () => bridge.called("sync_now").length === 1);
  expectPair(bridge.called("sync_now"), "photos", "retry from photos' banner");
  const tray = bridge.called("tray_action").filter((call) => call.args?.id === "tryAgain");
  if (tray.length)
    throw new Error("the retry went through the tray's row, which syncs every unpaused folder");
  await page.close();
});

await scenario(
  "a daemon that stops answering raises no banner about another folder's stale data",
  async () => {
    // `photos` is five minutes short of "nothing has synced for a day". The daemon stops, ten minutes pass, and
    // the roster the store still holds says `photos` last synced more than a day ago — which a banner built
    // from that roster would say as news. The folder is not being read any more; nothing is said about it.
    const bridge = new Bridge(
      { docs: [], photos: [] },
      { notifyPolicy: "only_when_needed", lastSync: nowSecs() - 60 },
    );
    bridge.lastSyncOf.photos = nowSecs() - 86_400 + 300;
    const page = await open(bridge);
    await page.bringToFront();
    await poll(page, bridge);
    await poll(page, bridge);
    if (bannersOf(bridge).length)
      throw new Error(`a banner before anything was wrong: ${JSON.stringify(bannersOf(bridge))}`);

    bridge.down = true;
    await page.evaluate(() => {
      window.__skewMs = 600_000;
    });
    await poll(page, bridge);
    await poll(page, bridge);
    if (bannersOf(bridge).length) {
      throw new Error(`the stopped daemon's stale roster raised ${JSON.stringify(bannersOf(bridge))}`);
    }

    // The positive control: the same roster, the same clock, a daemon that answers — and `photos` IS late.
    bridge.down = false;
    await pollForBanners(page, bridge, 1);
    const [banner] = bannersOf(bridge);
    if (banner.kind !== "outage" || banner.pair !== "photos") {
      throw new Error(`the live roster raised ${JSON.stringify(banner)}, expected an outage about photos`);
    }
    await page.close();
  },
);

await scenario("another folder's queue is fetched only while its summary counts one", async () => {
  const { bridge, page } = await openWithBanners({ docs: [], photos: [] });
  await poll(page, bridge);
  await poll(page, bridge);
  const reads = () => bridge.called("get_status").filter((call) => call.args?.pair === "photos").length;
  if (reads() !== 0) throw new Error(`photos' status was read ${reads()} time(s) while its queue was empty`);

  bridge.queues.photos = [permanent("p.txt")];
  await poll(page, bridge);
  await poll(page, bridge);
  const during = reads();
  if (during === 0) throw new Error("photos' queue was never read");

  bridge.queues.photos = [];
  await poll(page, bridge);
  await poll(page, bridge);
  const settled = reads();
  await poll(page, bridge);
  if (reads() !== settled) throw new Error("photos' status was still read after its queue drained");
  await page.close();
});

await scenario("at one folder a banner names none, and its buttons act as they always did", async () => {
  const { bridge, page } = await openWithBanners({ docs: [permanent("d.txt")] }, { names: ["docs"] });
  await pollForBanners(page, bridge, 1);
  const [banner] = bannersOf(bridge);
  if ("pair" in banner || banner.app !== "Drive Sync") {
    throw new Error(`a one-folder banner was ${JSON.stringify(banner)}`);
  }
  await clickBanner(page, { id: 1, kind: "deletion", action: "keep" });
  await until("the keep", () => bridge.called("keep").length === 1);
  expectPair(bridge.called("keep"), "docs", "a one-folder keep");
  await clickBanner(page, { id: 1, kind: "outage", action: "retry" });
  await until("the tray's retry", () =>
    bridge.called("tray_action").some((call) => call.args?.id === "tryAgain"),
  );
  if (bridge.called("sync_now").length)
    throw new Error("a one-folder retry was sent as a pair-addressed sync");
  await page.close();
});

// ---- what is known about a folder that is not on screen (#102 phase 5e, review round) ----------------------
//
// "Only what is known is said." The banner for a folder that is not on screen is built from its roster
// SUMMARY (a count, refreshed every poll) and from what the poll FETCHED for it (the list behind the count, and
// its conflict scan), which lands later. Two things went wrong when the second was treated as if it were as
// fresh as the first; both are visible only on the real page, because they depend on which of the poll's
// requests has come home when the notifier runs.

/** The banners as one comparable line each. */
const lines = (bridge) => bannersOf(bridge).map((b) => `${b.kind}|${b.pair ?? "-"}|${b.body}`);
const saveNotifier = (page) => page.evaluate(() => localStorage.getItem("notifier"));

await scenario(
  "a banner never names a file `Keep them` already kept, and the next one names only the new deletion",
  async () => {
    const { bridge, page } = await openWithBanners({ docs: [], photos: [permanent("x.txt")] });
    await pollForBanners(page, bridge, 1);
    if (!bannersOf(bridge)[0].body.includes("x.txt")) {
      throw new Error(`the first banner was ${JSON.stringify(bannersOf(bridge)[0])}, expected x.txt's`);
    }
    // `Keep them` on that banner. The daemon applies it, the queue is empty — and nothing refetches at 0.
    await clickBanner(page, { id: 1, kind: "deletion", action: "keep", pair: "photos" });
    await until("the keep", () => bridge.called("keep").length >= 1);
    bridge.queues.photos = [];
    await poll(page, bridge);
    await poll(page, bridge);

    // A NEW deletion, well after the window. The first poll that counts it runs the notifier before its read
    // has come home: whatever it says must not be about x.txt.
    await page.evaluate(() => {
      window.__skewMs = 120_000;
    });
    bridge.queues.photos = [permanent("y.txt")];
    const before = bannersOf(bridge).length;
    await poll(page, bridge);
    const early = bannersOf(bridge).slice(before);
    if (early.some((banner) => banner.body.includes("x.txt"))) {
      throw new Error(`a banner named the file that was already kept: ${JSON.stringify(early)}`);
    }

    await pollForBanners(page, bridge, before + 1);
    const banner = bannersOf(bridge)[before];
    if (!banner.body.includes("y.txt") || banner.body.includes("x.txt")) {
      throw new Error(`the new deletion's banner was ${JSON.stringify(banner)}`);
    }
    // And its button keeps y.txt, the file it names, and nothing else.
    const keepsBefore = bridge.called("keep").length;
    await clickBanner(page, { id: 2, kind: "deletion", action: "keep", pair: "photos" });
    await until("the second keep", () => bridge.called("keep").length > keepsBefore);
    const targets = bridge
      .called("keep")
      .slice(keepsBefore)
      .map((call) => call.args.target);
    if (targets.length !== 1 || targets[0] !== "y.txt") {
      throw new Error(`the second banner kept ${JSON.stringify(targets)}, expected y.txt alone`);
    }
    await page.close();
  },
);

await scenario(
  "while another folder's list is older than its count, the folder list counts the summary and `Keep them` keeps nothing",
  async () => {
    const { bridge, page } = await openWithBanners({ docs: [], photos: [permanent("x.txt")] });
    await pollForBanners(page, bridge, 1);
    const count = () =>
      page.evaluate(
        () => document.querySelector('.pair-row[data-pair="photos"] .pair-row-count')?.textContent,
      );

    // photos' queue grows to two, and the read that would say which two is still on its way. The list the
    // window holds is x.txt alone — fetched when the summary counted one.
    const release = bridge.hold("get_status", "photos");
    bridge.queues.photos = [permanent("y1.txt"), permanent("y2.txt")];
    await poll(page, bridge);
    await openList(page);
    if ((await count()) !== CHROME.chips.waiting(2)) {
      throw new Error(
        `the list counts ${JSON.stringify(await count())} from an old list, not the summary's two`,
      );
    }
    // `Keep them` on the banner acts on what is KNOWN to be the queue, which is nothing until it is read.
    await clickBanner(page, { id: 1, kind: "deletion", action: "keep", pair: "photos" });
    await settle(page);
    if (bridge.called("keep").length) {
      throw new Error(
        `Keep them kept ${JSON.stringify(bridge.called("keep").map((c) => c.args))} from an old list`,
      );
    }

    // The read comes home: now it is the queue, and the same press keeps it.
    release();
    await until("the list to be read", async () => (await count()) === CHROME.chips.waiting(2));
    await settle(page);
    await clickBanner(page, { id: 1, kind: "deletion", action: "keep", pair: "photos" });
    await until("the keeps", () => bridge.called("keep").length >= 2);
    const targets = bridge
      .called("keep")
      .map((call) => call.args.target)
      .sort();
    if (JSON.stringify(targets) !== JSON.stringify(["y1.txt", "y2.txt"])) {
      throw new Error(`Keep them kept ${JSON.stringify(targets)}, expected y1.txt and y2.txt`);
    }
    await page.close();
  },
);

for (const [what, queues, extra] of [
  ["a deletion in a folder that is not on screen", { docs: [], photos: [permanent("x.txt")] }, {}],
  [
    "a conflict in a folder that is not on screen",
    { docs: [], photos: [] },
    { conflicts: { docs: [], photos: [CONFLICT] } },
  ],
  ["a deletion in the folder on screen", { docs: [permanent("x.txt")], photos: [] }, {}],
  [
    "a conflict in the folder on screen",
    { docs: [], photos: [] },
    { conflicts: { docs: [CONFLICT], photos: [] } },
  ],
  ["a deletion at one folder", { docs: [permanent("x.txt")] }, { names: ["docs"] }],
]) {
  await scenario(`${what} that was already said is not said again by the next launch`, async () => {
    const bridge = new Bridge(queues, {
      notifyPolicy: "only_when_needed",
      lastSync: nowSecs() - 60,
      ...extra,
    });
    const first = await open(bridge);
    await first.bringToFront();
    await pollForBanners(first, bridge, 1);
    const saved = await saveNotifier(first);
    await first.close();
    const said = lines(bridge);

    // The next launch, a few minutes on: the same daemon, the same standing condition. The first poll counts
    // the queue and its read has not landed; the conflict scan has not run. Neither is "nothing there".
    const second = await open(bridge, "", { state: saved, skewMs: 120_000 });
    await second.bringToFront();
    for (let i = 0; i < 4; i += 1) await poll(second, bridge);
    if (lines(bridge).length !== said.length) {
      throw new Error(`launch 2 said it again: ${JSON.stringify(lines(bridge).slice(said.length))}`);
    }
    await second.close();
  });
}

await scenario(
  "a relaunch does not go quiet about a folder that is not on screen: a new deletion is said",
  async () => {
    const bridge = new Bridge(
      { docs: [], photos: [permanent("x.txt")] },
      { notifyPolicy: "only_when_needed", lastSync: nowSecs() - 60 },
    );
    const first = await open(bridge);
    await first.bringToFront();
    await pollForBanners(first, bridge, 1);
    const saved = await saveNotifier(first);
    await first.close();

    const second = await open(bridge, "", { state: saved, skewMs: 120_000 });
    await second.bringToFront();
    for (let i = 0; i < 3; i += 1) await poll(second, bridge);
    if (bannersOf(bridge).length !== 1)
      throw new Error(`launch 2 repeated: ${JSON.stringify(lines(bridge))}`);
    bridge.queues.photos = [permanent("x.txt"), permanent("y.txt")];
    await pollForBanners(second, bridge, 2);
    // The body names the first path only; the summary counts the queue, which is what changed.
    const banner = bannersOf(bridge)[1];
    if (!banner.summary.startsWith("2 files") || banner.pair !== "photos") {
      throw new Error(`the changed queue's banner was ${JSON.stringify(banner)}`);
    }
    await second.close();
  },
);

await scenario(
  "when the default folder is removed, the next folder's own conflict at the same path is still said",
  async () => {
    const bridge = new Bridge(
      { docs: [], photos: [] },
      {
        notifyPolicy: "only_when_needed",
        lastSync: nowSecs() - 60,
        conflicts: { docs: [CONFLICT], photos: [] },
      },
    );
    const page = await open(bridge);
    await page.bringToFront();
    await pollForBanners(page, bridge, 1);
    if (bannersOf(bridge)[0].pair !== "docs") {
      throw new Error(`the first banner was ${JSON.stringify(bannersOf(bridge)[0])}, expected docs'`);
    }

    // docs is removed and the daemon restarted: photos is the only folder, so the default, with a conflict of
    // its own at the very same relative path. Nothing about it has ever been said.
    await page.evaluate(() => {
      window.__skewMs = 120_000;
    });
    bridge.names = ["photos"];
    bridge.selected = "photos";
    bridge.conflicts = { photos: [CONFLICT] };
    for (let i = 0; i < 4 && bannersOf(bridge).length < 2; i += 1) await poll(page, bridge);
    if (bannersOf(bridge).length !== 2) {
      throw new Error(`photos' own conflict was held back by docs' memory: ${JSON.stringify(lines(bridge))}`);
    }
    await page.close();
  },
);

// ---- adding and removing folders (#102 phase 5c-2) ---------------------------------------------------
//
// THE ROAD THE ADD TAKES, driven on the real page against the scripted daemon: a dialog opened from the ⋯ menu, a
// name that follows the folder, a check that prices both sides and writes nothing, ONE `add_pair`, the restart, the
// wait for the daemon to LIST the folder, the selection, and a merge dialog that waits for the NEW folder's pass.
// What the bridge cannot be is the engine: the sentences it refuses with are the scenario's (`nameRefusals`,
// `survivors`), and the assertion is that the page quotes them.
//
//  70. THE WHOLE ROAD, in order, to the new folder's merge and no further.
//  71. A RESTART THAT FAILED shows the engine's reason in the dialog, offers the retry, and leaves the Settings bar's.
//  72. A NAME THE ENGINE REFUSES is quoted verbatim under the field, and nothing is sent.
//  73. A CHECK IS ABOUT THE TEXT IT WAS MADE FOR: editing after it puts `Check folders` back, and Enter does not add.
//  74. THE INDEX THE FOLDER WOULD RESUME is named in the confirmation, before anything is written.
//  75. A DIALOG WITH SOMETHING IN FLIGHT CANNOT BE LEFT: Esc is swallowed and the ✕ does nothing.
//  76. AN ADD THE ENGINE REFUSES is quoted, restarts nothing, and puts the check back.
//  77. A DAEMON THAT DOES NOT LIST THE FOLDER says so after the wait, and the folder is in the file.
//  78. THE LIST draws at two folders, not at one; a click on a row selects that folder.
//  79. REMOVING THE FIRST FOLDER names which one becomes the default; removing another does not.
//  80. THE ANSWER OF A REMOVAL is the command's own account, verbatim.
//  81. A SAVE AT TWO FOLDERS says what it costs all of them; at one it says what it always did.
//  82. WHAT WAS STAGED FOR ONE FOLDER SURVIVES CHOOSING ANOTHER FROM THE LIST and still saves to the first.
//  83. THE MENU OFFERS `Add folder…` AT EVERY COUNT.

/** The dialog's text, or "" when none is open. */
const dialogText = (page) => page.evaluate(() => document.querySelector(".dialog")?.innerText ?? "");
const hasDialog = (page) => page.evaluate(() => Boolean(document.querySelector(".dialog")));
/** What a dialog field holds, by its `data-field`. */
const fieldValue = (page, field) =>
  page.evaluate((name) => document.querySelector(`[data-field="${name}"]`)?.value ?? null, field);
/** Type into a dialog field, replacing what it holds. */
async function typeIntoField(page, field, text) {
  await page.focus(`[data-field="${field}"]`);
  await page.evaluate((name) => {
    const node = document.querySelector(`[data-field="${name}"]`);
    node.select();
  }, field);
  await page.keyboard.press("Backspace");
  if (text) await page.keyboard.type(text);
}
/** Is the button reading `label` there, and armed? */
const armed = (page, label) =>
  page.evaluate(
    (text) =>
      [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === text && !b.disabled),
    label,
  );

/** Open the add dialog from the ⋯ menu and fill the two folders in. */
async function fillAddDialog(page, local = "~/Photos", remote = "/Drive/Photos") {
  await press(page, "⋯");
  await press(page, FOLDERS.addFolder);
  await until("the add dialog", () => hasDialog(page));
  await typeIntoField(page, "folder-local", local);
  await typeIntoField(page, "folder-remote", remote);
}

/** Press `Check folders` and wait for the dialog to offer `Add folder`. */
async function checkTheFolders(page) {
  await until("an armed Check folders", () => armed(page, FOLDERS.add.check));
  await press(page, FOLDERS.add.check);
  await until("Add folder", () => armed(page, FOLDERS.add.add));
}

/** A daemon with ONE folder, which is the person the ⋯ menu's entry is for. */
const oneFolder = (extra = {}) => new Bridge({ docs: [] }, { names: ["docs"], selected: "docs", ...extra });

await scenario(
  "adding a folder checks, writes once, restarts, waits to be listed, selects it and merges against ITS counter",
  async () => {
    const bridge = oneFolder();
    // The folder on screen has run many passes and the new one none: its counter is not the new folder's, and a
    // merge that took the baseline from it would wait for a pass number the new folder never reaches.
    bridge.seqs.docs = 40;
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    // THE NAME FOLLOWS THE FOLDER until it is typed in: the engine's suggestion, not a regex of the page's.
    await until("the suggested name", async () => (await fieldValue(page, "folder-name")) === "photos");
    if (bridge.called("add_pair").length) throw new Error("typing wrote the file");

    await checkTheFolders(page);
    const shown = await dialogText(page);
    for (const sentence of ["1,204 files, 3.4 GB", "1,190 files", FOLDERS.add.noPreview]) {
      if (!shown.includes(sentence))
        throw new Error(`the checked dialog does not say ${JSON.stringify(sentence)}`);
    }
    // BOTH SIDES WERE PRICED, the remote one as a Drive path, and the check wrote nothing.
    const probes = bridge.called("probe_folder").map((call) => `${call.args.side}:${call.args.path}`);
    if (JSON.stringify(probes.sort()) !== JSON.stringify(["local:~/Photos", "remote:/Drive/Photos"])) {
      throw new Error(`the sides were measured as ${JSON.stringify(probes)}`);
    }
    if (bridge.called("add_pair").length) throw new Error("the check wrote the file");

    // The new folder is a folder that has never synced; the first-run takeover must not take the window.
    bridge.neverSynced = ["photos"];
    await press(page, FOLDERS.add.add);
    await until("the one add", () => bridge.called("add_pair").length === 1);
    expectPair(bridge.called("add_pair"), "photos", "add_pair");
    const sent = bridge.called("add_pair")[0].args.init;
    if (
      JSON.stringify(sent) !==
      JSON.stringify({ local_root: "~/Photos", remote_root: "/Drive/Photos", exclude: [] })
    ) {
      throw new Error(`add_pair carried ${JSON.stringify(sent)}`);
    }
    await until("the restart", () => bridge.called("restart_service").length === 1);
    if (bridge.called("restart_service")[0].args.onlyIfRunning !== true) {
      throw new Error("the restart was not `only if running`: an add is not a request to start syncing");
    }

    // THE MERGE DIALOG, for the folder the daemon now lists — and only then was it selected.
    await until("the merge dialog", async () => (await dialogText(page)).includes(ONBOARDING.progressTitle));
    const selections = bridge.called("select_pair");
    if (selections.length !== 1 || selections[0].args.name !== "photos") {
      throw new Error(`the selection was ${JSON.stringify(selections.map((c) => c.args))}`);
    }
    const order = bridge.calls.map((call) => call.cmd);
    if (
      order.indexOf("add_pair") > order.indexOf("restart_service") ||
      order.indexOf("restart_service") > order.indexOf("select_pair")
    ) {
      throw new Error(
        `the steps ran out of order: ${order.filter((c) => /add_pair|restart_service|select_pair/.test(c)).join(" ")}`,
      );
    }
    const merge = await dialogText(page);
    if (/nothing (was )?deleted/i.test(merge)) {
      throw new Error(
        `the merge of a folder nobody rehearsed claims nothing was deleted: ${JSON.stringify(merge)}`,
      );
    }
    if ((await pageText(page)).includes("Which two folders should match?")) {
      throw new Error("the first-run takeover took the window beside the new folder");
    }

    // ITS counter and nobody else's: another folder finishing a pass does not end the merge …
    bridge.seqs.docs = 99;
    await poll(page, bridge);
    if (!(await dialogText(page)).includes(ONBOARDING.progressTitle)) {
      throw new Error("another folder's pass ended the new folder's merge");
    }
    // … and its own pass does.
    bridge.finishPass("photos");
    await poll(page, bridge);
    await until("the merge to end", async () => !(await hasDialog(page)));
    await page.close();
  },
);

await scenario(
  "a restart that did not work is quoted in the dialog, retried from it, and left on the bar",
  async () => {
    const bridge = oneFolder();
    bridge.restartEnding = "never_stopped";
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    await checkTheFolders(page);
    await press(page, FOLDERS.add.add);
    await until("the engine's reason", async () => (await dialogText(page)).includes("it would not stop"));
    const said = await dialogText(page);
    if (!said.includes(SETTINGS.savedOldSettings("it would not stop"))) {
      throw new Error(`the failure is not the Settings save's sentence: ${JSON.stringify(said)}`);
    }
    if (!(await hasButton(page, SETTINGS.restart))) throw new Error("the dialog offers no way to restart");

    // The retry is NOT `only if running` — the failure may have stopped the daemon — and it carries on.
    bridge.restartEnding = "restarted";
    await press(page, SETTINGS.restart);
    await until("the retry", () => bridge.called("restart_service").length === 2);
    if (bridge.called("restart_service")[1].args.onlyIfRunning !== false) {
      throw new Error("the retry was `only if running`");
    }
    await until("the merge dialog", async () => (await dialogText(page)).includes(ONBOARDING.progressTitle));
    await page.close();
  },
);

await scenario("the Settings bar keeps the retry a failed add-restart left behind", async () => {
  const bridge = oneFolder();
  bridge.restartEnding = "never_stopped";
  const page = await open(bridge);
  await until("a first poll", () => bridge.called("get_status").length >= 1);
  await fillAddDialog(page);
  await checkTheFolders(page);
  await press(page, FOLDERS.add.add);
  await until("the failure", async () => (await dialogText(page)).includes("it would not stop"));
  await press(page, FOLDERS.remove.done);
  await until("the dialog to close", async () => !(await hasDialog(page)));
  await press(page, "Settings");
  await until("the bar's retry", () => hasButton(page, SETTINGS.restart));
  await page.close();
});

await scenario("a name the engine refuses is quoted under the field and nothing is sent", async () => {
  const bridge = oneFolder();
  const page = await open(bridge);
  await until("a first poll", () => bridge.called("get_status").length >= 1);
  await fillAddDialog(page);
  await until("the suggested name", async () => (await fieldValue(page, "folder-name")) === "photos");
  await typeIntoField(page, "folder-name", "docs");
  const sentence = "two `[[pair]]` tables are named `docs`";
  await until("the engine's sentence", async () => (await dialogText(page)).includes(sentence));
  if (await armed(page, FOLDERS.add.check)) throw new Error("Check folders is armed over a refused name");
  if (await hasButton(page, FOLDERS.add.add)) throw new Error("Add folder is offered over a refused name");
  if (bridge.called("add_pair").length) throw new Error("something was written");
  // A name the person typed is theirs: a later folder does not overwrite it.
  await typeIntoField(page, "folder-local", "~/Pictures");
  await settle(page);
  await until("a settled field", async () => (await fieldValue(page, "folder-name")) === "docs");
  await page.close();
});

await scenario(
  "a check is about the text it was made for, and Enter never adds an unchecked folder",
  async () => {
    const bridge = oneFolder();
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    await checkTheFolders(page);
    await typeIntoField(page, "folder-remote", "/Drive/Pictures");
    await until("Check folders back", () => armed(page, FOLDERS.add.check));
    if (await hasButton(page, FOLDERS.add.add)) throw new Error("Add folder survived an edit");
    await page.focus('[data-field="folder-remote"]');
    await page.keyboard.press("Enter");
    await until("a second check", () => bridge.called("probe_folder").length >= 4);
    if (bridge.called("add_pair").length) throw new Error("Enter added a folder that had not been checked");
    await page.close();
  },
);

await scenario(
  "the index a new folder would resume is named in the confirmation, before anything is written",
  async () => {
    const bridge = oneFolder();
    const message =
      "This folder already holds sync history from an earlier setup (~/Photos/.sync/sync_index.db); adding it resumes from that history. To start fresh instead, run proton-sync reset-index --yes --pair photos after adding.";
    bridge.survivors["~/Photos"] = {
      path: "~/Photos/.sync/sync_index.db",
      set_aside_pending: false,
      message,
    };
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    if ((await dialogText(page)).includes("earlier setup")) {
      throw new Error("the index was named before the folders were checked: it belongs to the confirmation");
    }
    await checkTheFolders(page);
    if (!(await dialogText(page)).includes(message)) throw new Error("the surviving index is not named");
    if (bridge.called("add_pair").length) throw new Error("naming it wrote the file");
    await page.close();
  },
);

await scenario("a dialog with an add in flight cannot be left, by Esc or by the ✕", async () => {
  const bridge = oneFolder();
  const page = await open(bridge);
  await until("a first poll", () => bridge.called("get_status").length >= 1);
  await fillAddDialog(page);
  await checkTheFolders(page);
  const release = bridge.hold("add_pair");
  await press(page, FOLDERS.add.add);
  await until("the add in flight", () => bridge.called("add_pair").length === 1);
  await page.keyboard.press("Escape");
  await page.evaluate(() => document.querySelector(".dialog-close")?.click());
  await settle(page);
  if (!(await hasDialog(page))) throw new Error("the dialog was closed with an add in flight");
  release();
  await until("the restart", () => bridge.called("restart_service").length === 1);
  await page.close();
});

await scenario(
  "an add the engine refuses is quoted, restarts nothing, and asks for a new check",
  async () => {
    const bridge = oneFolder();
    bridge.addRefusal = "the folder `/home/u/Photos` overlaps the folder of `docs`";
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    await checkTheFolders(page);
    await press(page, FOLDERS.add.add);
    await until("the refusal", async () =>
      (await dialogText(page)).includes("overlaps the folder of `docs`"),
    );
    if (bridge.called("restart_service").length) throw new Error("a refused add restarted the daemon");
    await until("Check folders back", () => armed(page, FOLDERS.add.check));
    await page.close();
  },
);

await scenario(
  "a daemon that restarts and does not list the folder says so, and the folder is in the file",
  async () => {
    const bridge = oneFolder();
    bridge.refuseToList = true;
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    await checkTheFolders(page);
    await press(page, FOLDERS.add.add);
    await until("the restart", () => bridge.called("restart_service").length === 1);
    for (let i = 0; i < 24; i += 1) await poll(page, bridge);
    await until("the sentence", async () =>
      (await dialogText(page)).includes(FOLDERS.add.notListed("photos")),
    );
    if (bridge.called("select_pair").length) throw new Error("a folder the daemon does not run was selected");
    await page.close();
  },
);

await scenario("the list is drawn at two folders and not at one, and a row chooses its folder", async () => {
  const two = new Bridge({ docs: [], photos: [] });
  const page = await open(two);
  await press(page, "Settings");
  await until("the list", () => page.$(".folders-list"));
  const rows = await page.evaluate(() =>
    [...document.querySelectorAll(".folders-row")].map((row) => ({
      name: row.querySelector(".folders-row-name")?.textContent,
      selected: row.classList.contains("is-selected"),
    })),
  );
  if (
    JSON.stringify(rows) !==
    JSON.stringify([
      { name: "docs", selected: true },
      { name: "photos", selected: false },
    ])
  ) {
    throw new Error(`the list was ${JSON.stringify(rows)}`);
  }
  if (!(await pageText(page)).includes(FOLDERS.list.editing("docs"))) {
    throw new Error("the list does not say which folder the settings below are for");
  }
  await page.evaluate(() =>
    document.querySelector('.folders-row[data-folder="photos"] .folders-row-main').click(),
  );
  await until("the selection", () => two.called("select_pair").length === 1);
  if (two.called("select_pair")[0].args.name !== "photos")
    throw new Error("a click on photos' row chose another folder");
  await page.close();

  const one = oneFolder();
  const alone = await open(one);
  await press(alone, "Settings");
  await until("the settings", () => hasButton(alone, SETTINGS.save));
  await settle(alone);
  if (await alone.$(".folders-list")) throw new Error("a list was drawn for one folder");
  if ((await pageText(alone)).includes(FOLDERS.list.title.toUpperCase()))
    throw new Error("the list's title is on screen");
  await alone.close();
});

// ---- a folder that has not had its turn (the live report, and #455) -----------------------------------------
//
// Three folders, the default one (`documents`) deep in a 25 minute full walk. Passes are serialized, so the
// other two have not been reached: the window showed `photos` as `Nothing has synced yet` under "Open Drive
// Sync to choose your two folders", and the list called `videos` `up to date`. These are driven with the
// payloads the real Rust sends for exactly that (`NEVER_SYNCED`), not with a stand-in's guess at them.

const THREE = Object.freeze(["documents", "photos", "videos"]);

/** The three folders of the live report, answered with the build's own payloads for `moment`. */
const threeFolders = (moment, selected = "photos") =>
  new Bridge(
    { documents: [], photos: [], videos: [] },
    { names: THREE, selected, replay: NEVER_SYNCED[moment] },
  );

/** What the Settings list says for each folder: the word after its name. */
const settingsWords = (page) =>
  page.evaluate(() =>
    Object.fromEntries(
      [...document.querySelectorAll(".folders-row")].map((row) => [
        row.dataset.folder,
        row.querySelector(".folders-row-state")?.textContent ?? null,
      ]),
    ),
  );

/** Whatever of `texts` the page says, for a message that shows what was on screen. */
const saysAny = (shown, texts) => texts.filter((text) => shown.includes(text));

await scenario(
  "a folder waiting behind a running pass says it is waiting, and for whom, and never that it is up to date",
  async () => {
    const bridge = threeFolders("running");
    const page = await open(bridge);
    await until("the hero", async () => (await pageText(page)).includes(TRAY.waitingTitle("documents")));
    const shown = await pageText(page);
    // THE FOLDER ON SCREEN: the reason it waits, the folder it waits for, and nothing that was said before.
    if (!shown.includes(TRAY.waitingSub("photos", "documents"))) {
      throw new Error(`the hero does not say why photos waits: ${JSON.stringify(shown)}`);
    }
    const wrong = saysAny(shown, [
      MAIN.settled,
      TRAY.nothingSyncedYet,
      "choose your two folders",
      MAIN.settledSubTime(""),
    ]);
    if (wrong.length)
      throw new Error(`the window says ${JSON.stringify(wrong)} about photos: ${JSON.stringify(shown)}`);
    if ((await chipText(page)) !== CHROME.chips.queued) {
      throw new Error(`the chip is ${JSON.stringify(await chipText(page))}, not ${CHROME.chips.queued}`);
    }
    // It can still be synced now or paused, by name — a folder that waits is not a folder with nothing to do.
    for (const label of [MAIN.syncNow, TRAY.pausePair("photos")]) {
      if (!(await hasButton(page, label))) throw new Error(`the hero has no ${JSON.stringify(label)}`);
    }
    // The takeover is the first-folder flow and stays shut.
    if (/step 1 of 2/.test(shown)) throw new Error("the first-run takeover opened over a running app");

    // THE FOLDERS BESIDE IT, in the header's list: the folder that is running, and two that wait. The one the
    // window was not about used to read `up to date`. The list is 280px and its rows spend it on the folder's
    // own name (`waiting for documents` left `photos` three letters), so the word is `waiting` and the row
    // carries the whole as its tooltip.
    await openList(page);
    const rows = await listRows(page);
    const words = Object.fromEntries(THREE.map((name) => [name, rows[name]?.[0]]));
    const short = {
      documents: CHROME.pair.states.running,
      photos: CHROME.pair.waiting,
      videos: CHROME.pair.waiting,
    };
    if (JSON.stringify(words) !== JSON.stringify(short)) {
      throw new Error(`the list says ${JSON.stringify(words)}, not ${JSON.stringify(short)}`);
    }
    const tips = await page.evaluate(() =>
      [...document.querySelectorAll(".pair-row")].map((row) => [
        row.dataset.pair,
        row.querySelector(".pair-row-state").getAttribute("title"),
      ]),
    );
    if (
      JSON.stringify(tips) !==
      JSON.stringify([
        ["documents", null],
        ["photos", CHROME.pair.waitingFor("documents")],
        ["videos", CHROME.pair.waitingFor("documents")],
      ])
    ) {
      throw new Error(`the rows' tooltips are ${JSON.stringify(tips)}`);
    }
    // And the names are READABLE: no row has squeezed the folder's own name to an ellipsis.
    const clipped = await page.evaluate(() =>
      [...document.querySelectorAll(".pair-row-name")]
        .filter((name) => name.scrollWidth > name.clientWidth)
        .map((name) => name.textContent),
    );
    if (clipped.length) throw new Error(`the list clips the names ${JSON.stringify(clipped)}`);
    await page.keyboard.press("Escape");

    // AND THE SETTINGS LIST, which has the room and names the folder.
    await press(page, "Settings");
    await until("the list", () => page.$(".folders-list"));
    const settings = await settingsWords(page);
    const wanted = {
      documents: CHROME.pair.states.running,
      photos: CHROME.pair.waitingFor("documents"),
      videos: CHROME.pair.waitingFor("documents"),
    };
    if (JSON.stringify(settings) !== JSON.stringify(wanted)) {
      throw new Error(`the Settings list says ${JSON.stringify(settings)}, not ${JSON.stringify(wanted)}`);
    }
    await page.close();
  },
);

await scenario(
  "the folder that read up to date is drawn as waiting too once it is the one on screen",
  async () => {
    const bridge = threeFolders("running", "documents");
    const page = await open(bridge);
    // First the default folder, which is the one running: it is syncing, and says nothing about waiting.
    await until("documents is syncing", async () => (await chipText(page)) === CHROME.chips.syncing);
    if ((await pageText(page)).includes(TRAY.waitingTitle("documents"))) {
      throw new Error("the folder that is running is waiting for itself");
    }
    await select(page, bridge, "videos");
    await until("videos waits", async () => (await pageText(page)).includes(TRAY.waitingTitle("documents")));
    const shown = await pageText(page);
    if (!shown.includes(TRAY.waitingSub("videos", "documents"))) {
      throw new Error(`the hero does not name videos: ${JSON.stringify(shown)}`);
    }
    if (shown.includes(MAIN.settled)) throw new Error("videos was drawn as up to date");
    await until("the buttons name videos", () => hasButton(page, TRAY.pausePair("videos")));
    await page.close();
  },
);

await scenario(
  "with nothing running yet the folders say they are starting, in the window and in the tray",
  async () => {
    const bridge = threeFolders("starting");
    const page = await open(bridge);
    await until("the hero", async () => (await pageText(page)).includes(TRAY.startingTitle));
    const shown = await pageText(page);
    if (!shown.includes(TRAY.startingSub("photos"))) {
      throw new Error(`the hero does not say photos starts on its own: ${JSON.stringify(shown)}`);
    }
    if (saysAny(shown, [MAIN.settled, TRAY.nothingSyncedYet, "choose your two folders"]).length) {
      throw new Error(`the window says what it said before: ${JSON.stringify(shown)}`);
    }
    if ((await chipText(page)) !== CHROME.chips.starting) {
      throw new Error(`the chip is ${JSON.stringify(await chipText(page))}, not ${CHROME.chips.starting}`);
    }
    await openList(page);
    const words = Object.values(await listRows(page)).map((row) => row[0]);
    if (
      JSON.stringify(words) !==
      JSON.stringify([CHROME.pair.states.queued, CHROME.pair.states.queued, CHROME.pair.states.queued])
    ) {
      throw new Error(`the list says ${JSON.stringify(words)}`);
    }
    await page.close();

    // THE TRAY PANEL, for the default folder it describes: the same two sentences, no button that sends a person
    // to the window to choose folders they have chosen, and a pause row for every folder.
    const panel = await open(threeFolders("starting"), "?surface=tray");
    await until("the panel", async () => (await pageText(panel)).includes(TRAY.startingTitle));
    const text = await pageText(panel);
    if (!text.includes(TRAY.startingSub("documents"))) {
      throw new Error(`the panel does not say documents starts on its own: ${JSON.stringify(text)}`);
    }
    if (text.includes(TRAY.nothingSyncedYetSub) || text.includes(MAIN.compact.upToDate)) {
      throw new Error(`the panel says what it said before: ${JSON.stringify(text)}`);
    }
    if (await panel.$(".compact-action-btn"))
      throw new Error("the panel offers a button for a folder that only has to start");
    for (const name of THREE) {
      if (!(await hasButton(panel, TRAY.pausePair(name)))) {
        throw new Error(`the panel has no ${JSON.stringify(TRAY.pausePair(name))}`);
      }
    }
    await panel.close();
  },
);

await scenario(
  "a folder the file lists beside a never-synced one is told it starts on its own, not to choose folders",
  async () => {
    // The daemon runs ONE folder (the file lists two and it has not restarted onto the second), so it derives
    // the wizard's state for it; the window knows of two and keeps the wizard shut. #455: the sub-line it draws
    // was the tray's, which sends a person to the window they are in.
    const bridge = new Bridge(
      { docs: [] },
      { names: ["docs"], roster: ["docs", "photos"], neverSynced: ["docs"] },
    );
    const page = await open(bridge);
    await until("the hero", async () => (await pageText(page)).includes(TRAY.nothingSyncedYet));
    const shown = await pageText(page);
    if (!shown.includes(MAIN.firstRunSub))
      throw new Error(`the sub-line is missing: ${JSON.stringify(shown)}`);
    if (shown.includes("choose your two folders") || shown.includes(TRAY.nothingSyncedYetSub)) {
      throw new Error(`the window sends the person to choose folders: ${JSON.stringify(shown)}`);
    }
    if (/step 1 of 2/.test(shown)) throw new Error("the first-run takeover opened over a running app");
    await page.close();
  },
);

/** Open Settings at two folders and press `Remove` on the row of `name`. */
async function pressRemoveOn(page, name) {
  await press(page, "Settings");
  await until("the list", () => page.$(".folders-list"));
  await page.evaluate(
    (folder) =>
      document
        .querySelector(
          `.folders-row[data-folder="${folder}"] .folders-row > .btn, .folders-row[data-folder="${folder}"] button[aria-label^="Remove"]`,
        )
        ?.click(),
    name,
  );
  await until("the confirmation", async () => (await dialogText(page)).includes(FOLDERS.remove.title(name)));
}

await scenario(
  "removing the first folder says which one becomes the default; removing another does not",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await pressRemoveOn(page, "docs");
    const first = await dialogText(page);
    for (const sentence of [
      FOLDERS.remove.stops("docs"),
      FOLDERS.remove.keeps,
      FOLDERS.remove.history(bridge.setAsideDir),
      FOLDERS.remove.becomesDefault("photos"),
    ]) {
      if (!first.includes(sentence))
        throw new Error(`the confirmation does not say ${JSON.stringify(sentence)}`);
    }
    await press(page, FOLDERS.add.cancel);
    await until("the dialog to close", async () => !(await hasDialog(page)));
    if (bridge.called("remove_pair").length) throw new Error("Cancel removed a folder");

    await pressRemoveOn(page, "photos");
    const second = await dialogText(page);
    if (second.includes("becomes the default folder")) {
      throw new Error("removing the second folder names a new default: the first is still the default");
    }
    if (!second.includes(FOLDERS.remove.keeps)) throw new Error("the second confirmation lost its sentences");
    await page.close();
  },
);

await scenario(
  "a removal shows the command's own account, verbatim, and is acted on for the folder it named",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await pressRemoveOn(page, "photos");
    await select(page, bridge, "docs");
    await press(page, FOLDERS.remove.confirm);
    await until("the removal", () => bridge.called("remove_pair").length === 1);
    expectPair(bridge.called("remove_pair"), "photos", "remove_pair");
    const account = `The sync history of 'photos' was moved to ${bridge.setAsideDir}/photos-1, outside every sync folder. Your files were not touched.`;
    await until("the account", async () => (await dialogText(page)).includes(account));
    if (!(await dialogText(page)).includes(FOLDERS.remove.removed("photos"))) {
      throw new Error("the answer does not say the folder was removed");
    }
    await press(page, FOLDERS.remove.done);
    await until("the dialog to close", async () => !(await hasDialog(page)));
    await page.close();
  },
);

await scenario(
  "a save at two folders says what it costs all of them, and at one says what it always did",
  async () => {
    const two = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS });
    const page = await open(two);
    await openDeletions(page);
    await pressCard(page, SETTINGS.askNever);
    const note = await until("the bar's sentence", async () => {
      const text = await pageText(page);
      return text.includes(FOLDERS.saveRestartsAll(2)) ? text : null;
    });
    if (!note) throw new Error("no sentence");
    await page.close();

    const one = new Bridge({ docs: [] }, { names: ["docs"], selected: "docs", configs: SETTINGS_CONFIGS });
    const alone = await open(one);
    await openDeletions(alone);
    await pressCard(alone, SETTINGS.askNever);
    await settle(alone);
    if ((await pageText(alone)).includes("Saving restarts syncing for all")) {
      throw new Error("the sentence about all folders is on screen with one");
    }
    await alone.close();
  },
);

await scenario(
  "choosing another folder from the list keeps what was staged, and it still saves to the first",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { configs: SETTINGS_CONFIGS });
    const page = await open(bridge);
    await openDeletions(page);
    await pressCard(page, SETTINGS.askNever);
    await press(page, SETTINGS.tabs.folders);
    await until("the list", () => page.$(".folders-list"));
    await page.evaluate(() =>
      document.querySelector('.folders-row[data-folder="photos"] .folders-row-main').click(),
    );
    await until("photos selected", () => bridge.selected === "photos");
    await poll(page, bridge);
    // The folder chosen has nothing staged: docs' edit is not applied to it, and Save is not armed for it.
    if (await armed(page, SETTINGS.save)) throw new Error("docs' staged edit arms Save on photos");
    await page.evaluate(() =>
      document.querySelector('.folders-row[data-folder="docs"] .folders-row-main').click(),
    );
    await until("docs selected", () => bridge.selected === "docs");
    await poll(page, bridge);
    await press(page, SETTINGS.save);
    await until("the write", () => bridge.called("write_config").length === 1);
    expectPair(bridge.called("write_config"), "docs", "write_config");
    await page.close();
  },
);

await scenario("the menu offers Add folder… at one folder and at two", async () => {
  for (const bridge of [oneFolder(), new Bridge({ docs: [], photos: [] })]) {
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await press(page, "⋯");
    await until("the entry", () => hasButton(page, FOLDERS.addFolder));
    await page.close();
  }
});

// ---- the folder dialogs, left by a shortcut or a banner (review of #450, F1) -------------------------------------
//
//  84. A DIALOG WITH AN ADD IN FLIGHT is not left by Ctrl+, Ctrl+F or a banner's `Review` either: the keypress is
//      consumed, nothing moves, and the add goes on to its merge.
//  85. A DIALOG WITH A REMOVAL IN FLIGHT is not left by them, and when the daemon answers its account is on screen.
//  86. A DIALOG THAT IS LEFT TAKES ITS FOLDER STATE WITH IT, by a shortcut or by Esc: a check a keystroke had queued
//      is not asked afterwards. (Listed after 87 and 88 in the file.)
//  87. NOTHING OPENS OVER A DIALOG WITH SOMETHING IN FLIGHT: not another folder's Remove, not the menu's Add folder,
//      not a failed save's `Save refused`.
//  88. CTRL+F ON THE ACTIVITY SCREEN does not move focus into the lookup behind such a dialog.

/** A key with Ctrl held, as the shell's shortcuts read it (`onKeydown`). */
async function pressWithCtrl(page, key) {
  await page.keyboard.down("Control");
  await page.keyboard.press(key);
  await page.keyboard.up("Control");
}

/** The route of the door that is lit, or null when none is. */
const litDoor = (page) =>
  page.evaluate(() => document.querySelector('[data-route][aria-current="page"]')?.dataset.route ?? null);

/** Every way the window is told to go somewhere else while a dialog is up. */
async function tryEveryWayOut(page) {
  await pressWithCtrl(page, ",");
  await pressWithCtrl(page, "f");
  await clickBanner(page, { id: 1, kind: "deletion", action: "review", pair: "docs" });
  await settle(page);
}

await scenario(
  "Ctrl+, Ctrl+F and a banner's Review do not leave a dialog with an add in flight, and the add goes on to its merge",
  async () => {
    const bridge = oneFolder();
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    await checkTheFolders(page);
    const door = await litDoor(page);
    const release = bridge.hold("add_pair");
    await press(page, FOLDERS.add.add);
    await until("the add in flight", () => bridge.called("add_pair").length === 1);
    await tryEveryWayOut(page);
    if (!(await hasDialog(page)))
      throw new Error("a shortcut or a banner closed the dialog of an add in flight");
    if ((await litDoor(page)) !== door) {
      throw new Error(`the window moved to ${await litDoor(page)} under a dialog that cannot be left`);
    }
    release();
    await until("the restart", () => bridge.called("restart_service").length === 1);
    await until("the merge dialog", async () => (await dialogText(page)).includes(ONBOARDING.progressTitle));
    await page.close();
  },
);

await scenario(
  "Ctrl+, Ctrl+F and a banner's Review do not leave a dialog with a removal in flight, and its account is shown",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await pressRemoveOn(page, "photos");
    const release = bridge.hold("remove_pair");
    await press(page, FOLDERS.remove.confirm);
    await until("the removal in flight", () => bridge.called("remove_pair").length === 1);
    await tryEveryWayOut(page);
    if (!(await hasDialog(page)))
      throw new Error("a shortcut or a banner closed the dialog of a removal in flight");
    if ((await litDoor(page)) !== "settings") {
      throw new Error(`the window moved to ${await litDoor(page)} under a dialog that cannot be left`);
    }
    release();
    const account = `The sync history of 'photos' was moved to ${bridge.setAsideDir}/photos-1, outside every sync folder. Your files were not touched.`;
    await until("the account", async () => (await dialogText(page)).includes(account));
    await page.close();
  },
);

await scenario(
  "nothing opens over a dialog with something in flight: not another folder's Remove, not the menu's Add folder, not a failed save's",
  async () => {
    // A removal in flight. The scrim stops a pointer; a programmatic click is what reaches the buttons behind it.
    const removing = new Bridge({ docs: [], photos: [] });
    const page = await open(removing);
    await pressRemoveOn(page, "photos");
    const release = removing.hold("remove_pair");
    await press(page, FOLDERS.remove.confirm);
    await until("the removal in flight", () => removing.called("remove_pair").length === 1);
    await page.evaluate(() =>
      document.querySelector('.folders-row[data-folder="docs"] button[aria-label^="Remove"]')?.click(),
    );
    await press(page, "⋯");
    await press(page, FOLDERS.addFolder);
    await settle(page);
    if (!(await dialogText(page)).includes(FOLDERS.remove.removing("photos"))) {
      throw new Error(
        `another dialog took the place of the removal: ${JSON.stringify(await dialogText(page))}`,
      );
    }
    release();
    const account = `The sync history of 'photos' was moved to ${removing.setAsideDir}/photos-1, outside every sync folder. Your files were not touched.`;
    await until("the account", async () => (await dialogText(page)).includes(account));
    await page.close();

    // An add in flight, and a save that was already out when it started and is refused now.
    const adding = new Bridge({ docs: [] }, { names: ["docs"], selected: "docs", configs: SETTINGS_CONFIGS });
    const other = await open(adding);
    await openDeletions(other);
    await pressCard(other, SETTINGS.askNever);
    const releaseSave = adding.hold("write_config");
    await press(other, SETTINGS.save);
    await until("the save to leave", () => adding.called("write_config").length === 1);
    await fillAddDialog(other);
    await checkTheFolders(other);
    const releaseAdd = adding.hold("add_pair");
    await press(other, FOLDERS.add.add);
    await until("the add in flight", () => adding.called("add_pair").length === 1);
    adding.writeRefusal = "disk full";
    releaseSave();
    await settle(other);
    const shown = await dialogText(other);
    if (shown.includes(SETTINGS.refusedTitleUnknown) || !shown.includes(FOLDERS.add.title)) {
      throw new Error(`a failed save took the place of the add in flight: ${JSON.stringify(shown)}`);
    }
    releaseAdd();
    await until("the merge dialog", async () => (await dialogText(other)).includes(ONBOARDING.progressTitle));
    await other.close();
  },
);

await scenario(
  "Ctrl+F on the Activity screen does not move focus into the lookup behind a dialog with an add in flight",
  async () => {
    const bridge = oneFolder();
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await pressWithCtrl(page, "f"); // the Activity screen, with the lookup focused
    await until("the Activity screen", async () => (await litDoor(page)) === "activity");
    await fillAddDialog(page);
    await checkTheFolders(page);
    const release = bridge.hold("add_pair");
    await press(page, FOLDERS.add.add);
    await until("the add in flight", () => bridge.called("add_pair").length === 1);
    await page.evaluate(() => document.activeElement?.blur());
    await pressWithCtrl(page, "f");
    await settle(page);
    const inLookup = await page.evaluate(() => Boolean(document.activeElement?.isContentEditable));
    if (inLookup) throw new Error("Ctrl+F moved focus into the lookup behind the dialog");
    release();
    await page.close();
  },
);

await scenario(
  "a dialog that is left takes its folder state with it, by a shortcut or by Esc: a check a keystroke queued is not asked",
  async () => {
    for (const way of ["Ctrl+,", "Esc"]) {
      const bridge = oneFolder();
      const page = await open(bridge);
      await until("a first poll", () => bridge.called("get_status").length >= 1);
      await fillAddDialog(page);
      await settle(page);
      // The last keystroke queued a check; leave before it is asked.
      await typeIntoField(page, "folder-local", "~/Pictures");
      if (way === "Esc") await page.keyboard.press("Escape");
      else await pressWithCtrl(page, ",");
      const asked = bridge.called("check_add_pair").length;
      await until("the dialog to go", async () => !(await hasDialog(page)));
      await delay(600);
      await settle(page);
      if (bridge.called("check_add_pair").length !== asked) {
        throw new Error(`${way}: the engine was asked about an add whose dialog had been left`);
      }
      await page.close();
    }
  },
);

// ---- the review's smaller findings (review of #450) -------------------------------------------------------------
//
//  89. THE LAST FOLDER'S CONFIRMATION SAYS WHY it cannot be removed (F5).
//  90. AN ADD THAT FINISHED AN EARLIER REMOVAL says so, and rests on `Done` before the merge dialog (F9).
//  91. REMOVING THE SELECTED FOLDER moves the selection first: nothing asks for the removed name afterwards.
//  92. RETURNING TO THE TEXT A CHECK WAS MADE FOR brings back what the check found (J11).
//  93. A LATE ANSWER FOR OLD TEXT does not replace the engine's answer for the text on screen (J12).
//
// ---- the final fix round of the review (#450) --------------------------------------------------------------------
//
//  94. REMOVING THE SELECTED FOLDER MOVES THE SELECTION TO A FOLDER THE SERVICE LISTS, not to the first the file holds:
//      the file's first remaining folder may be one the service does not run, and `select_pair` is refused for it.
//      When no folder can be selected the config read names none, and the removed name is still never asked for.
//  95. A FAILED SAVE THAT REPLACES THE ADD DIALOG takes the check a keystroke had queued with it.

await scenario(
  "the confirmation of the last folder says why it cannot be removed, and removes nothing",
  async () => {
    // The file lists one folder and the daemon runs two (a restart the edit has not reached), so the list is
    // drawn with a single row — the one state in which `Remove` is offered on the only folder.
    const bridge = new Bridge({ docs: [], photos: [] }, { roster: ["docs"] });
    const page = await open(bridge);
    await pressRemoveOn(page, "docs");
    const shown = await dialogText(page);
    if (!shown.includes(FOLDERS.remove.last("docs"))) {
      throw new Error(`the confirmation does not say why: ${JSON.stringify(shown)}`);
    }
    if (shown.includes(FOLDERS.remove.stops("docs"))) {
      throw new Error("the confirmation promises a removal that cannot happen");
    }
    if (await armed(page, FOLDERS.remove.confirm))
      throw new Error("Remove folder is armed for the only folder");
    if (bridge.called("remove_pair").length) throw new Error("something was removed");
    await page.close();
  },
);

await scenario(
  "an add that finished an earlier removal says so, and rests on Done before the merge dialog",
  async () => {
    const bridge = oneFolder();
    const line =
      "The earlier removal of 'old' moved its history to /home/u/.local/state/proton-sync/removed-pairs/old-1.";
    bridge.addSettled = [line];
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await fillAddDialog(page);
    await checkTheFolders(page);
    await press(page, FOLDERS.add.add);
    await until("the dialog resting on the listed folder", async () =>
      (await dialogText(page)).includes(FOLDERS.add.listed("photos")),
    );
    const resting = await dialogText(page);
    if (!resting.includes(line))
      throw new Error(`the earlier removal is not told: ${JSON.stringify(resting)}`);
    if (resting.includes(ONBOARDING.progressTitle)) throw new Error("the merge dialog replaced the account");
    if (bridge.called("select_pair").length !== 1) throw new Error("the new folder was not selected");
    // A folder the service lists finished everything: its sentence is not drawn in the amber of a save that did not.
    const amber = await page.evaluate(() =>
      Boolean(document.querySelector(".folders-status")?.classList.contains("is-cost")),
    );
    if (amber) throw new Error("the listed folder's sentence is drawn as a cost");
    // Nothing is in flight now: the dialog may be left, and Done carries on to the merge.
    await press(page, FOLDERS.remove.done);
    await until("the merge dialog", async () => (await dialogText(page)).includes(ONBOARDING.progressTitle));
    await page.close();
  },
);

await scenario(
  "removing the selected folder moves the selection first, so nothing asks for the removed name",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { selected: "photos" });
    bridge.strict = true;
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await pressRemoveOn(page, "photos");
    await press(page, FOLDERS.remove.confirm);
    await until("the account", async () =>
      (await dialogText(page)).includes(FOLDERS.remove.removed("photos")),
    );
    await settle(page);
    const after = bridge.calls.slice(bridge.calls.findIndex((call) => call.cmd === "remove_pair") + 1);
    const asked = after.filter((call) => call.cmd === "read_config" && call.args?.pair === "photos");
    if (asked.length) throw new Error(`the removed folder was asked for by name ${asked.length} time(s)`);
    const selected = after.filter((call) => call.cmd === "select_pair").map((call) => call.args.name);
    if (JSON.stringify(selected) !== JSON.stringify(["docs"])) {
      throw new Error(
        `the selection moved to ${JSON.stringify(selected)}, expected the first remaining folder`,
      );
    }
    if ((await pageText(page)).includes("no folder pair named")) {
      throw new Error("the Settings screen is showing the refusal of the removed name");
    }
    await page.close();

    // A folder that is not the selected one goes without the selection moving at all.
    const other = new Bridge(
      { docs: [], photos: [], music: [] },
      // `photos` is selected and is not the first: a removal that moved the selection to the first remaining
      // folder for no reason would visibly move it to `docs`.
      { names: ["docs", "photos", "music"], selected: "photos" },
    );
    other.strict = true;
    const second = await open(other);
    await until("a first poll", () => other.called("get_status").length >= 1);
    await pressRemoveOn(second, "music");
    await press(second, FOLDERS.remove.confirm);
    await until("the account", async () =>
      (await dialogText(second)).includes(FOLDERS.remove.removed("music")),
    );
    await settle(second);
    if (other.called("select_pair").length) throw new Error("removing another folder moved the selection");
    await second.close();
  },
);

await scenario("returning to the text a check was made for brings back what the check found", async () => {
  const bridge = oneFolder();
  const page = await open(bridge);
  await until("a first poll", () => bridge.called("get_status").length >= 1);
  await fillAddDialog(page);
  await checkTheFolders(page);
  const prices = ["1,204 files, 3.4 GB", "1,190 files"];
  const priced = async () => {
    const text = await dialogText(page);
    return prices.map((price) => text.includes(price));
  };
  if (JSON.stringify(await priced()) !== "[true,true]")
    throw new Error("the checked dialog is missing a price");
  // Away from it: unchecked, and nothing is claimed about a folder that was not measured.
  await typeIntoField(page, "folder-local", "~/Pictures");
  await until("Check folders back", () => armed(page, FOLDERS.add.check));
  if ((await priced()).some(Boolean)) throw new Error("a price is on screen for text that was not measured");
  // And back to exactly what was measured: the check is good again, and so is what it found.
  await typeIntoField(page, "folder-local", "~/Photos");
  await until("Add folder again", () => armed(page, FOLDERS.add.add));
  await settle(page);
  if (JSON.stringify(await priced()) !== "[true,true]") {
    throw new Error(`the check is armed again but its prices are ${JSON.stringify(await priced())}`);
  }
  // A check that the person types away from while it is being made leaves the earlier findings alone too:
  // it was overtaken, so it found nothing to put in their place.
  const releaseProbe = bridge.hold("probe_folder");
  await typeIntoField(page, "folder-local", "~/Pictures");
  // The name follows the folder's, and a check made while it moves is overtaken by it: let it settle first.
  await until("the suggested name", async () => (await fieldValue(page, "folder-name")) === "pictures");
  await until("Check folders", () => armed(page, FOLDERS.add.check));
  await press(page, FOLDERS.add.check);
  await until("the second check's measuring to start", () => bridge.called("probe_folder").length >= 3);
  await typeIntoField(page, "folder-local", "~/Photos");
  releaseProbe();
  await until("Add folder again", () => armed(page, FOLDERS.add.add));
  await settle(page);
  if (JSON.stringify(await priced()) !== "[true,true]") {
    throw new Error(`an overtaken check took the prices away: ${JSON.stringify(await priced())}`);
  }
  await page.close();
});

await scenario(
  "a late answer for old text does not replace the engine's answer for the text on screen",
  async () => {
    const bridge = oneFolder();
    const page = await open(bridge);
    await until("a first poll", () => bridge.called("get_status").length >= 1);
    await press(page, "⋯");
    await press(page, FOLDERS.addFolder);
    await until("the add dialog", () => hasDialog(page));
    // The answer to the first text is held back; the second text's is not.
    const release = bridge.hold("check_add_pair");
    await typeIntoField(page, "folder-local", "~/Photos");
    await typeIntoField(page, "folder-remote", "/Drive/Photos");
    await until("the first question to leave", () => bridge.called("check_add_pair").length === 1);
    await typeIntoField(page, "folder-name", "docs");
    const sentence = "two `[[pair]]` tables are named `docs`";
    await until("the engine's sentence for the second text", async () =>
      (await dialogText(page)).includes(sentence),
    );
    release(); // …and now the first answer arrives, late
    await settle(page);
    await settle(page);
    if (!(await dialogText(page)).includes(sentence)) {
      throw new Error("a late answer for old text replaced the engine's refusal of the name on screen");
    }
    await page.close();
  },
);

await scenario(
  "removing the selected folder moves the selection to a folder the service lists, and names none when none can be selected",
  async () => {
    // The file lists `docs` first and the service does not run it (added, and not restarted onto yet), so the
    // selection cannot go there: it is refused, and the selection stayed on the removed name for the config
    // read to ask for.
    const running = () => {
      const bridge = new Bridge(
        { docs: [], photos: [], music: [] },
        { names: ["photos", "music"], roster: ["docs", "photos", "music"], selected: "photos" },
      );
      bridge.strict = true;
      bridge.refuseToList = true;
      return bridge;
    };
    const removePhotos = async (bridge) => {
      const page = await open(bridge);
      await until("a first poll", () => bridge.called("get_status").length >= 1);
      await pressRemoveOn(page, "photos");
      await press(page, FOLDERS.remove.confirm);
      await until("the account", async () =>
        (await dialogText(page)).includes(FOLDERS.remove.removed("photos")),
      );
      await settle(page);
      const after = bridge.calls.slice(bridge.calls.findIndex((call) => call.cmd === "remove_pair") + 1);
      if (after.some((call) => call.cmd === "read_config" && call.args?.pair === "photos")) {
        throw new Error("the removed folder was asked for by name");
      }
      if ((await pageText(page)).includes("no folder pair named")) {
        throw new Error("the Settings screen is showing the refusal of the removed name");
      }
      return { page, after };
    };

    const listed = running();
    const first = await removePhotos(listed);
    const moved = first.after.filter((call) => call.cmd === "select_pair").map((call) => call.args.name);
    if (JSON.stringify(moved) !== JSON.stringify(["music"])) {
      throw new Error(
        `the selection went to ${JSON.stringify(moved)}, expected the first folder the service lists`,
      );
    }
    if (listed.selected !== "music") throw new Error(`the selection is ${listed.selected}, expected music`);
    await first.page.close();

    // The one folder the service lists is refused too (it stopped running it since the window last heard): the
    // selection stays, the read names no folder, and Rust answers for the default one.
    const refused = running();
    refused.selectRefused = new Set(["music"]);
    const second = await removePhotos(refused);
    const tried = second.after.filter((call) => call.cmd === "select_pair").map((call) => call.args.name);
    if (JSON.stringify(tried) !== JSON.stringify(["music"])) {
      throw new Error(`the selection was tried on ${JSON.stringify(tried)}, expected only the listed folder`);
    }
    // The first read after the removal is the one the removal makes: it names no folder. (Later reads are the
    // window's own, for whichever folder the service's reply then puts on screen.)
    const reads = second.after.filter((call) => call.cmd === "read_config");
    if (!reads.length || reads[0].args?.pair) {
      throw new Error(
        `the config was first read for ${JSON.stringify(reads[0]?.args?.pair)}, expected no name`,
      );
    }
    await second.page.close();
  },
);

await scenario(
  "a failed save that replaces the add dialog takes the check a keystroke had queued with it",
  async () => {
    // The add dialog is only being typed in, so nothing is in flight and the failed save's `Save refused` may take
    // its place. A dialog that is replaced lets go of what it held exactly as one that is closed does.
    const bridge = new Bridge({ docs: [] }, { names: ["docs"], selected: "docs", configs: SETTINGS_CONFIGS });
    const page = await open(bridge);
    await openDeletions(page);
    await pressCard(page, SETTINGS.askNever);
    const releaseSave = bridge.hold("write_config");
    await press(page, SETTINGS.save);
    await until("the save to leave", () => bridge.called("write_config").length === 1);
    await fillAddDialog(page);
    await settle(page);
    // The last keystroke queued a check; the save is refused before it is asked.
    await typeIntoField(page, "folder-local", "~/Pictures");
    bridge.writeRefusal = "disk full";
    releaseSave();
    await until(
      "the add dialog to be replaced",
      async () => !(await dialogText(page)).includes(FOLDERS.add.title),
    );
    if (!(await hasDialog(page))) throw new Error("the failed save drew no dialog of its own");
    const asked = bridge.called("check_add_pair").length;
    await delay(600);
    await settle(page);
    if (bridge.called("check_add_pair").length !== asked) {
      throw new Error("the engine was asked about an add whose dialog had been replaced");
    }
    await page.close();
  },
);

await browser.close();
server.close();

if (failures.length) {
  console.error(`\nfidelity:pairs — ${failures.length} failure(s):\n`);
  for (const failure of failures) console.error(`  ${failure}\n`);
  console.error("A write must act on the pair it was drawn for, not on whichever is selected when it runs.");
  process.exit(1);
}
console.log("fidelity:pairs — every write acted on the pair it was drawn for");
