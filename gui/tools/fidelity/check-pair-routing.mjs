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
// Sixty-five scenarios (the first sixteen are phases 5a-2, 5d and 5b-1's; forty-three are phase 5c-1's —
// twenty-one built with the selector, the nineteen its review added and the three its second review added — and the last six
// are phase 5e's, the notifications; both below the list). The first four are each a way a write can land on a different pair than the one it was
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
//   5. A PAIR THAT HAS NEVER SYNCED. At two folders the first-run wizard (whose `Next` writes
//      top-level roots a `[[pair]]` file refuses) must stay shut, and the window must say the pair has
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
// WHAT IT CANNOT SEE: it scripts the bridge, so it proves the facade and the screens agree with each
// other, not that the real Rust agrees with either (that is `selection_tests.rs`); and it drives the
// window, except the tray panel scenarios above, which drive `?surface=tray` through the same bridge.

import puppeteer from "puppeteer";
import { serve } from "./serve.mjs";
import { CHROME, CONFLICTS, DELETIONS, MAIN, PLAN, SETTINGS, TRAY } from "../../src/js/ui/copy.js";
import { EMPTY_CONFIG } from "../../src/js/api.js";

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
    } = {},
  ) {
    this.names = names;
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

  /** What Rust answers when the socket does not (`status_payload`): no reply, no roster, the app's own selection. */
  unreachable() {
    return { state: "unreachable", error: "connect: no such file or directory", selected: this.selected };
  }

  /**
   * The reply to a status read. `name` is the pair it DESCRIBES — the one the request named, else the
   * selected one, as Rust answers — and `selected` is the app's choice, which is not the same thing.
   */
  status(name = this.selected) {
    const never = (pair) => this.neverSynced.includes(pair);
    const pairs = this.names.map((pair) => ({
      ...summaryOf(pair, this.queues[pair].length),
      last_sync_epoch_secs: this.lastSyncOf[pair] ?? this.lastSync,
      ...(never(pair) ? { last_sync_epoch_secs: null } : {}),
      ...(this.states[pair]?.summary ?? {}),
      ...(this.isPaused(pair) ? { paused: true } : {}),
    }));
    return {
      // What `derive_state` says: a reachable daemon that has never synced THIS pair — or the state the
      // scenario gave it (`states`), which is how a folder is failed, paused or unavailable on screen.
      state: never(name) ? "firstRun" : this.isPaused(name) ? "paused" : (this.states[name]?.state ?? "idle"),
      selected: this.selected,
      ...(this.pairUnknown ? { pair_unknown: this.pairUnknown } : {}),
      pairs,
      pair_states: this.names.map((pair) => ({
        name: pair,
        state: this.isPaused(pair) ? "paused" : (this.states[pair]?.state ?? "idle"),
        rank: this.isPaused(pair) ? 1 : (this.states[pair]?.rank ?? 0),
      })),
      response: {
        status: "running",
        paused: this.isPaused(name),
        syncing: false,
        reconcile_seq: 1,
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
        const pair = args?.pair ?? this.selected;
        return {
          ...EMPTY_CONFIG,
          exists: true,
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
          if (this.roster) this.names = [...this.roster];
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

/** A fresh page wired to a fresh bridge. `query` is the page's own (`?surface=tray` opens the tray panel). */
async function open(bridge, query = "") {
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
  "at two pairs a never-synced pair is drawn as such, and the first-run takeover stays shut",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { neverSynced: ["docs"] });
    const page = await open(bridge);
    await until("the hero", async () => (await pageText(page)).includes(TRAY.nothingSyncedYet));
    const shown = await pageText(page);
    if (shown.includes(MAIN.settled))
      throw new Error(`a pair that has never synced was drawn as "${MAIN.settled}"`);
    if (!shown.includes(TRAY.nothingSyncedYetSub))
      throw new Error("the second sentence of the never-synced hero is missing");
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
  if (!shown.includes(TRAY.nothingSyncedYet)) {
    throw new Error(
      `the panel does not describe the default pair (docs, never synced): ${JSON.stringify(shown)}`,
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
        states: { photos: { state: "failed", rank: 4, summary: { last_error: "boom", pending_changes: 3 } } },
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
  rank: 4,
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

await browser.close();
server.close();

if (failures.length) {
  console.error(`\nfidelity:pairs — ${failures.length} failure(s):\n`);
  for (const failure of failures) console.error(`  ${failure}\n`);
  console.error("A write must act on the pair it was drawn for, not on whichever is selected when it runs.");
  process.exit(1);
}
console.log("fidelity:pairs — every write acted on the pair it was drawn for");
