// Adding, listing and removing folders (#102 phase 5c-2): the MODEL behind the Settings list, the add
// dialog and the removal confirmation. Pure — no DOM, no `api`, no store — so every decision in it is a
// function a test can call, and `app.js` is left with the part that cannot be pure: the order things
// are asked in, and what is on screen while they are.
//
// THREE RULES THIS FILE HOLDS, each one a way the flow could say something false:
//
//   1. A CHECK IS ABOUT THE INPUTS IT WAS MADE FOR. The dialog asks the engine whether the add would go
//      ahead (`check_add_pair`) and prices both folders (`probe_folder`), and every answer is filed
//      with a key of the inputs it answered. Typing after that makes the key differ, so a `Check
//      folders` that passed for `~/Photos` cannot turn the button into `Add folder` for `~/Pictures`:
//      the add button exists only while the key it was checked at is the key on screen.
//
//   2. THE ENGINE'S SENTENCE IS QUOTED, NEVER WRITTEN. A name the engine refuses, a relative folder, an
//      overlap, the account of where a removed folder's history went: all of them arrive as text from
//      the command that made the decision, and none of it passes through the copy deck (voice rule 4).
//      What this file builds is the sentences AROUND them.
//
//   3. A LIST OF FOLDERS IS THE FILE'S, NOT THE DAEMON'S. The Settings list is made from the settings
//      file's roster (in file order — the first is the default folder), with each folder's state taken
//      from the daemon where the daemon runs it. A folder only the file knows is drawn, and says it is
//      not running yet; a list built from the daemon's would hide the folder a restart has not reached.

import { FOLDERS } from "./ui/copy.js";
import { selectorRows, stateWordOf } from "./ui/selector.js";

// ------------------------------------------------------------------------------- the add dialog ----

/** The phases in which something is in flight and the dialog may not be closed or edited. */
export const BUSY_PHASES = ["adding", "restarting", "waiting"];

/**
 * The phases that END the dialog's work and leave it to be read: the folder is in the settings and the
 * daemon does not run it (`added`), the restart or the wait did not work (`unresolved`), or the daemon lists
 * the folder and an earlier removal the add finished has something to tell (`listed`, review of #450, F9).
 * Nothing is in flight in any of them, so each may be left; each has the one button, `Done`.
 */
export const SETTLED_PHASES = ["added", "unresolved", "listed"];

/** A fresh add dialog: nothing typed, nothing asked. */
export function blankAdd() {
  return {
    // form | checking | adding | restarting | waiting | added | unresolved | listed
    phase: "form",
    name: "",
    // Has the person typed in the name field? Until they have, the field follows the suggestion made
    // from the folder's name; after, it is theirs and a suggestion never overwrites it.
    nameTouched: false,
    local: "",
    remote: "",
    rules: [],
    draft: "",
    // The last `check_add_pair` reply and the key of the inputs it answered; `null` before one.
    pre: null,
    preKey: null,
    // What `probe_folder` found, per side: a probe reply, `{ error }`, or `null` for not asked. Filed under
    // the key of the inputs the CHECK was made at (`checkedKey`), which is why editing a root empties it.
    probes: { local: null, remote: null },
    checkedKey: null,
    // Bumped by every check, so an answer that arrives after the person has typed on is dropped.
    seq: 0,
    // The add command's own refusal (its sentence, verbatim), or what a failed restart left behind.
    error: null,
    // The accounts of earlier removals this add finished (`add_pair`'s `settled_earlier`, review of #450, F9):
    // history that was moved, or a pending move that was dropped, as a side effect of adding a folder. Said in
    // the dialog from the moment the add answers, and the reason the dialog RESTS on `listed` instead of going
    // straight to the merge: a move of someone's files is not a line to flash past.
    settled: [],
    // The merge to open when the person presses `Done` on `listed`: `{ pair, seq, waits }`, read from the new
    // folder's own summary at the moment the daemon first listed it, exactly as the direct route reads it.
    merge: null,
    ending: null,
    reason: "",
    // Polls counted while the add waits for the restarted daemon to list the folder (`waitStepOf`). The
    // merge that follows has its own record (`app.js`'s `folderMerge`), which holds the NEW folder's
    // `reconcile_seq` as it was when the daemon first listed it — read from its own summary and nowhere else.
    waits: 0,
    // The status-request clock at the last poll counted while waiting: a wait is counted in polls, and a
    // render happens for many other reasons.
    seenIssue: 0,
  };
}

/** The inputs a check is made for. Rules are not among them: they ride along with the add, they are not checked. */
export const addKeyOf = (flow) => JSON.stringify([flow.name.trim(), flow.local.trim(), flow.remote.trim()]);

/**
 * What the add dialog draws and what its button does, from the flow alone.
 *
 * `fresh` is the whole of rule 1: the engine's answer is used only while it answers what is typed now.
 * Between a keystroke and the next answer nothing is claimed — the name field shows no error and the
 * button is merely not yet `Add folder` — rather than a refusal for a name that is no longer there.
 */
export function addViewOf(flow) {
  const key = addKeyOf(flow);
  const fresh = flow.preKey === key && flow.pre != null;
  const pre = fresh ? flow.pre : null;
  const busy = BUSY_PHASES.includes(flow.phase);
  const checking = flow.phase === "checking";
  const settled = SETTLED_PHASES.includes(flow.phase);
  const filled = Boolean(flow.name.trim() && flow.local.trim() && flow.remote.trim());
  // A name error is drawn once something has been typed or the field is not empty: a dialog that opens
  // with an empty name must not open with the engine's sentence about an empty name under it.
  const nameError = pre?.name_error && (flow.nameTouched || flow.name.trim() !== "") ? pre.name_error : null;
  const refusal = pre?.name_error ? null : (pre?.refusal ?? null);
  const blocked = Boolean(pre?.name_error || pre?.refusal);
  const checked = flow.checkedKey === key && !blocked;
  let primary = "check";
  if (busy) primary = "busy";
  else if (settled) primary = "done";
  else if (checked) primary = "add";
  return {
    key,
    fresh,
    busy,
    checking,
    settled,
    filled,
    nameError,
    refusal,
    blocked,
    checked,
    primary,
    // `Check folders` needs a filled form that nothing has refused; `Add folder` is armed by the check
    // itself; the rest are not buttons to press.
    primaryEnabled:
      primary === "add" || primary === "done" || (primary === "check" && filled && !blocked && !checking),
    // Where the dialog may be left: never mid-flight. The file is written before the restart, so closing
    // there would leave a folder added and nothing watching for it to appear.
    closable: !busy,
    // The surviving index and the warnings are the CONFIRMATION's: drawn once the check has passed.
    survivor: checked ? (pre?.surviving_index ?? null) : null,
    warnings: checked ? (pre?.warnings ?? []) : [],
  };
}

/**
 * One side's price line, from a probe reply.
 *
 * `null` for a side not measured yet. A failure carries the reason as it came. The remote side has no
 * size to report (a remote listing exposes none), and a walk that stopped at its bound, or could not
 * read every directory, is a floor and says `at least`. A local side whose bytes are known and whole
 * says both.
 */
export function priceOf(side, probe) {
  if (!probe) return null;
  if (probe.error) return { failed: true, text: FOLDERS.add.notMeasured(probe.error) };
  const files = probe.files ?? 0;
  const atLeast = Boolean(probe.truncated) || (probe.unreadable_directories ?? 0) > 0;
  if (side === "local" && probe.bytes != null && !atLeast) {
    return { failed: false, text: FOLDERS.add.priceLocal(files, probe.bytes) };
  }
  return { failed: false, text: FOLDERS.add.priceRemote(files, atLeast) };
}

/** A Drive path as the daemon's listing wants it: absolute. `Drive/X` and `/Drive/X` are one location. */
export const remotePathForProbe = (remote) => {
  const trimmed = remote.trim();
  return trimmed.startsWith("/") ? trimmed : `/${trimmed}`;
};

/** The staged skip rules plus one, unless it is empty or already there (a duplicate is not a second row). */
export function withRule(rules, draft) {
  const pattern = draft.trim();
  return pattern && !rules.includes(pattern) ? [...rules, pattern] : rules;
}

/**
 * The request `add_pair` takes, from the flow. Roots are trimmed here and again by the command, which
 * is the one that decides; the rules are the staged list, in the order they were added.
 */
export const addRequestOf = (flow) => ({
  local_root: flow.local.trim(),
  remote_root: flow.remote.trim(),
  exclude: [...flow.rules],
});

/**
 * The status reply, but only when it is ABOUT `pair`: otherwise `null`, which says nothing.
 *
 * A just-added folder's merge is judged by that folder's own pass counter. For a poll or two after the
 * selection moves the window may still hold the previous folder's reply, whose `reconcile_seq` is a different
 * counter on a different scale — read as the new folder's it would end the merge at once (a higher number) or
 * never (a lower one). Reading `pair` off the reply, and nothing else, is the whole rule.
 */
export const replyAbout = (reply, pair) => (reply && reply.pair === pair ? reply : null);

/**
 * Has the daemon listed the new folder yet — and if not, has the wait run out?
 *
 * One answer per poll while the dialog waits: `select` once the name is in the daemon's list (never
 * before: choosing a folder the daemon does not run is a selection nothing acts on), `timeout` after
 * `limit` polls without it, else keep waiting. The list is the daemon's own (`pairs[]`), because the
 * file lists the folder from the moment it is written and says nothing about whether it runs.
 */
export function waitStepOf({ name, listed, waits, limit }) {
  if (listed.includes(name)) return "select";
  return waits >= limit ? "timeout" : "wait";
}

/** How many polls (about two seconds apart) a restarted daemon has to list the folder that was added. */
export const WAIT_LIMIT = 20;

// ---------------------------------------------------------------------------- the Settings list ----

/**
 * The rows of the Folders tab's list, in FILE order (rule 3).
 *
 * `roster` is `read_config.pairs` — `{ name, local_root, remote_root }` as the file writes them — and
 * `pairs` the daemon's own summaries. Paths come from the file where it has them (the text a person
 * wrote, `~` and all) and from the daemon's otherwise; the state word is the selector's, through the
 * one `selectorRows`, so a folder reads the same in the list as in the popover. A folder the daemon
 * runs no longer or not yet says `not running yet` instead of a state — and only while the daemon is
 * answering, because a stopped daemon runs nothing and `unreachable` is what every row already says.
 */
export function listRows({
  roster = [],
  pairs = [],
  pairStates = [],
  selected = null,
  reachable = true,
} = {}) {
  const entries = roster.length > 0 ? roster : pairs;
  const summaries = new Map(pairs.map((pair) => [pair.name, pair]));
  const states = selectorRows({
    pairs: entries.map((entry) => ({ name: entry.name })),
    pairStates,
    selected,
    reachable,
  });
  return entries.map((entry, index) => {
    const summary = summaries.get(entry.name);
    const listed = summaries.has(entry.name);
    const word =
      reachable && pairs.length > 0 && !listed ? FOLDERS.list.notRunning : stateWordOf(states[index].state);
    return {
      name: entry.name,
      local: entry.local_root ?? summary?.local_root ?? "",
      remote: entry.remote_root ?? summary?.remote_root ?? "",
      selected: entry.name === selected,
      word,
    };
  });
}

// ---------------------------------------------------------------------------- the remove dialog ----

/** The state of a removal: confirming, running, or answered. */
export function blankRemove(pair) {
  return { pair, phase: "confirm", reply: null, error: null };
}

/**
 * What the confirmation says (decision D8), from the file's roster.
 *
 * Four sentences, in the order the maintainer gave them, the fourth only when the removed folder is the
 * FIRST — because the first is the default one, which every command that names none and every older
 * client means, and removing it moves that meaning to the next. `newDefault` is the next in FILE order,
 * the same one the engine promotes. Removing the last folder is refused before anything is asked
 * (`last`), as the engine refuses it: a config with no folder has nothing to do.
 *
 * `setAsideDir` is where the app moves a removed folder's history, named so it can be found afterwards;
 * `null` when the app has no state directory on this machine, in which case the sentence names the
 * place by what it is and the answer that follows says what happened.
 */
export function removalOf({ name, roster, setAsideDir = null }) {
  const names = roster.map((entry) => entry.name);
  const index = names.indexOf(name);
  const first = index === 0;
  const newDefault = first ? (names[1] ?? null) : null;
  const last = names.length <= 1;
  const lines = [
    FOLDERS.remove.stops(name),
    FOLDERS.remove.keeps,
    FOLDERS.remove.history(setAsideDir ?? "the app's state folder"),
  ];
  if (newDefault) lines.push(FOLDERS.remove.becomesDefault(newDefault));
  return {
    title: FOLDERS.remove.title(name),
    // The only folder says why its button is disabled and promises nothing: none of the four sentences is
    // true of a removal that cannot happen (review of #450, F5).
    lines: last ? [FOLDERS.remove.last(name)] : lines,
    first,
    newDefault,
    known: index >= 0,
    last,
  };
}

/**
 * What the removal dialog says once the command has answered, as sentences (voice rule 4 — the command's
 * own account is quoted, not rewritten).
 *
 * The first is always true of an answer at all: the folder is out of the settings. Then the account of
 * its history — `moved to X`, `nothing to move`, or pending with the reason — which is the command's
 * `set_aside.message`, whatever outcome it tags; then any earlier removal the same call finished; then,
 * when the restart it made left something unresolved, that, in `restartNote`'s words (the caller builds it
 * from the Settings save's sentences, so a restart that failed reads the same wherever it is said).
 */
export function accountOf(reply, { name, restartNote = null } = {}) {
  const lines = [FOLDERS.remove.removed(name ?? reply?.pair ?? "")];
  if (reply?.set_aside?.message) lines.push(reply.set_aside.message);
  lines.push(...settledLinesOf(reply));
  if (restartNote) lines.push(restartNote);
  return lines;
}

/**
 * The accounts of the earlier removals a command finished first (`settled_earlier[].message`), quoted. One
 * reader for the two commands that carry them: `remove_pair` shows them in its answer, and `add_pair` — which
 * settles what an earlier removal could not finish before it looks at the request — in the add dialog
 * (review of #450, F9). An entry with no sentence is nothing to say.
 */
export function settledLinesOf(reply) {
  const lines = [];
  for (const earlier of reply?.settled_earlier ?? []) {
    if (earlier?.message) lines.push(earlier.message);
  }
  return lines;
}
