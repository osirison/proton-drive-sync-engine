// The four triggers (C6, #179) — what interrupts, and what stays silent.
//
// PURE, AND DRIVEN FROM THE STATUS POLL. `decide` takes the world and the last thing we said and
// answers with an event or `null`; `app.js` calls it on every reply and hands the result to
// `ui/notification.js`. Nothing here touches the DOM, the clock or the network, so every rule below
// is testable in Node — which matters more here than usual, because the failure modes are "said
// nothing when files were about to go" and "said the same thing forty times".
//
// TWELVE CATEGORIES STAY SILENT and they are not listed in the code, because they are not branches:
// every sync pass, every file, every retry, every scheduled sweep reaches `decide` inside the status
// reply and produces `null` by falling off the end. `11a Rules` is the sheet that says so, and it is
// rendered from the same `NOTIFY.rules` a person reads in Settings.

import { severityOfItem } from "./ui/rows.js";

/** Highest first. A more serious event may interrupt a quieter one inside the coalescing window. */
const SEVERITY = { deletion: 3, outage: 2, conflict: 1, firstSync: 0 };

/**
 * "Coalesce within a 30-second window" (`11-notifications.md` §Grouping).
 *
 * Read as a RATE and not as a delay: the first event of a burst is shown immediately, and anything
 * arriving behind it inside the window replaces that banner rather than adding one. Delaying the
 * first would mean holding the permanent-deletion warning for half a minute, which is the one
 * banner whose whole justification is that silence costs files.
 */
export const COALESCE_MS = 30_000;

/** "Nothing has synced for a day". */
export const OUTAGE_AFTER_SECS = 86_400;

/** What each policy card lets through. `never` is the empty set and changes nothing else. */
const ALLOWED = {
  only_when_needed: new Set(["deletion", "conflict", "firstSync", "outage"]),
  only_permanent_deletions: new Set(["deletion"]),
  never: new Set(),
};

// ---------------------------------------------------------------------------- folders ----
//
// ONE NOTIFIER, SEVERAL FOLDERS (#102 phase 5e). What is remembered about a folder is keyed by the folder;
// what is rate-limited is the BANNER, and there is one banner on a screen whatever folder it is about
// (`11-notifications.md`: "never stack more than one Drive Sync banner"). So the memory splits and the
// window does not:
//
//   · PER FOLDER: what was said (`said`), whether this install watched the first sync (`sawUnsynced`) and
//     the newest sync it saw (`lastSeenSync`).
//   · GLOBAL: when anything was last shown, and what (`lastAt`, `lastKind`, `lastPair`).
//
// THE DEFAULT FOLDER KEEPS THE SHAPE EVERY SAVED STATE ALREADY HAS: bare kinds in `said`, the two
// witnesses at the top. A state written before folders existed is therefore read as the default folder's
// with no migration, and a second folder added later does not make the first forget what it said. Every
// OTHER folder is `kind@name` in `said` and `@name` in `seen`; the `@` is outside a folder's alphabet
// (`[A-Za-z0-9._-]`), so such a key can be neither a kind nor another folder's, and a folder called
// `constructor` or `__proto__` is an ordinary key rather than a property of the object.
//
// WHICH FOLDER IS THE DEFAULT IS PART OF THE STATE (`owner`). The bare keys are the default folder's, and a
// conflict's signature is only its relative paths — so when the default folder is removed and the next one
// becomes the default, reading the bare keys as the new default's would silence a conflict it never announced
// because another folder once announced one at the same path. `decide` therefore remembers whose the bare keys
// are, and when the default changes it moves each folder's memory to where that folder now lives: the removed
// folder's is dropped, a folder that is still listed keeps its own under `kind@name`, and the folder that
// became the default takes its `kind@name` memory into the bare keys.

/** The key `kind` is remembered under for the folder kept as `home` — `null` is the default folder. */
export const keyOf = (kind, home) => (home == null ? kind : `${kind}@${home}`);

/** The key a folder's witnesses are kept under in `state.seen`. */
const seenKey = (home) => `@${home}`;

/** What is known about a folder this install has never watched. */
const UNSEEN = Object.freeze({ sawUnsynced: false, lastSeenSync: null });

/** The state that has to survive a restart. Serialisable on purpose — `app.js` keeps it in storage. */
export const emptyState = () => ({
  /** key (see `keyOf`) → the signature of the last thing we said about it. */
  said: {},
  /** When we last showed anything, in ms, and about what — GLOBAL, because the banner is. */
  lastAt: 0,
  lastKind: null,
  /** The folder that banner was about, kept as `home`: `null` is the default folder (and every state saved before folders). */
  lastPair: null,
  /**
   * The NAME of the default folder the bare keys above belong to, or `null` when no live roster has said yet — a
   * state saved before this field existed, read as the current default's own. `decide` fills it in from the
   * roster's first entry (the folder an unaddressed request means) and re-homes the memory when it changes.
   */
  owner: null,
  /** The two witnesses below are the DEFAULT folder's; this is the same pair of facts for every other folder, by `@name`. */
  seen: {},
  /**
   * Whether this GUI has ever seen the daemon with no successful sync behind it.
   *
   * THE FIRST-SYNC BANNER NEEDS A WITNESS, not a value. `last_sync_epoch_secs` being set says
   * nothing about whether the first sync just finished — it is set on every successful pass, for
   * ever. Installing the GUI on a machine that has been syncing for a year would otherwise announce
   * `Both sides now match` as though the wait had just ended. So the banner fires only where this
   * process (or a previous one, through storage) actually watched the transition.
   *
   * AND A LIVE `null` IS NOT THAT WITNESS ON ITS OWN. `ControlShared::new` starts `last_sync` at
   * `None`, so every daemon RESTART answers `null` until its next successful pass — on an
   * established install that would make a witness out of a service restart and announce the first
   * sync again. It counts only when nothing has ever been seen (`lastSeenSync == null`), which is
   * the difference between "this daemon has not synced yet" and "this machine never has".
   */
  sawUnsynced: false,
  /**
   * The newest `last_sync_epoch_secs` this GUI has seen.
   *
   * AN UNREACHABLE DAEMON SENDS NO REPLY AT ALL, so `response` is null on every tick and the outage
   * trigger — which is a claim about how long it has been — would have had nothing to measure
   * against for the one outage most worth naming: a daemon that died a day ago. Remembered here
   * instead, and used only when the live reply cannot answer.
   */
  lastSeenSync: null,
});

/**
 * A saved state, read back. Anything that is not the shape of one is an empty state, and a state saved
 * BEFORE FOLDERS (no `lastPair`, no `seen`) is the default folder's own: it gets the new fields at their
 * empty values and keeps everything else, so nothing it had said is said again.
 *
 * Shape-checked rather than trusted: a half-written or older value must not make `decide` throw inside
 * the status poll, which would take the whole window's refresh down with it.
 *
 * `typeof null === "object"` PASSES THE FIRST CHECK, and that is safe rather than overlooked — reviewed
 * and reproduced, because it is the obvious place to assume otherwise. Object spread of `null` is a
 * no-op by specification, so `{ said: null }` resolves to `said: {}`, which is the empty state this
 * function would have returned anyway. It does not throw, and neither does `{ said: [] }`.
 *
 * `seen` IS FILTERED ENTRY BY ENTRY, because it is read by key: a value that is not `{ sawUnsynced,
 * lastSeenSync }` would make a folder look as though it had been watched when nothing had.
 */
export function restoreState(saved) {
  if (!saved || typeof saved !== "object" || typeof saved.said !== "object") return emptyState();
  const seen = {};
  if (saved.seen && typeof saved.seen === "object" && !Array.isArray(saved.seen)) {
    for (const [key, value] of Object.entries(saved.seen)) {
      if (!key.startsWith("@") || !value || typeof value !== "object") continue;
      seen[key] = {
        sawUnsynced: value.sawUnsynced === true,
        lastSeenSync: Number.isFinite(value.lastSeenSync) ? value.lastSeenSync : null,
      };
    }
  }
  return {
    ...emptyState(),
    ...saved,
    said: { ...saved.said },
    lastPair: typeof saved.lastPair === "string" ? saved.lastPair : null,
    owner: typeof saved.owner === "string" && saved.owner ? saved.owner : null,
    seen,
  };
}

/**
 * Permanent = the file leaves this computer for good.
 *
 * `severityOf` RATHER THAN A SECOND TEST, and its default is why: it answers `permanent` for
 * anything that is not the literal `remote`, so a direction nobody anticipated warns rather than
 * stays silent. A `direction === "local"` here would be the same rule written the fail-open way,
 * in the one trigger where being wrong costs files. S3 made this call for the screen (DEVIATIONS
 * §76); one implementation, not two that can drift.
 */
const isPermanent = (deletion) => severityOfItem(deletion) === "permanent";

// NUL, WRITTEN AS AN ESCAPE. A separator has to be something a path cannot contain, or two queues
// with different members could sign the same — and a literal control character in the source makes
// git call the file binary and every gate stay green (`check-sources.mjs` exists for that).
const sig = (parts) => parts.slice().sort().join("\u0000");

/**
 * Files at stake across a queue of permanent deletions, or null when one of them cannot be counted.
 *
 * A directory's `subtree_files` is the daemon's own count of what is under it (#208); a file is one
 * file. `null` PROPAGATES rather than being skipped — a banner saying `4 files` about a queue whose
 * uncounted folder holds a thousand is worse than one saying `2 folders`, which is the fallback the
 * caller keeps.
 */
function subtreeFileCount(deletions) {
  let total = 0;
  for (const deletion of deletions) {
    if (deletion.entity_kind !== "directory") {
      total += 1;
      continue;
    }
    if (deletion.subtree_files == null) return null;
    total += Number(deletion.subtree_files);
  }
  return total;
}

/**
 * What the world would interrupt about, in severity order. Pure: no clock, no policy, no memory.
 *
 * `nowSecs` is passed rather than read so the outage threshold is testable at a boundary.
 */
export function candidates({ response, conflicts = [], daemonState = null, lastSeenSync = null }, nowSecs) {
  const out = [];
  const deletions = (response?.pending_deletions ?? []).filter(isPermanent);
  if (deletions.length) {
    const paths = deletions.map((d) => String(d.path));
    // The noun only where the queue agrees. A mixed queue is `files`, which is true of the group.
    const kinds = new Set(deletions.map((d) => d.entity_kind));
    const entity = kinds.size === 1 && [...kinds][0] === "directory" ? "folder" : null;
    out.push({
      kind: "deletion",
      paths,
      entity,
      // HOW MANY FILES WOULD ACTUALLY GO, which is the banner's drawn count (#208) and not the
      // queue's length: one row can be a folder holding a thousand of them. A file counts as
      // itself, a folder as its subtree — and `null` the moment any folder cannot be counted (an
      // older daemon), because a total missing one row's contents is not a total.
      files: subtreeFileCount(deletions),
      // The fingerprint, not the path: an approval is pinned to one, so a path whose queued deletion
      // was decided and came back is genuinely a new thing to say.
      signature: sig(deletions.map((d) => `${d.path}\u0001${d.fingerprint ?? ""}`)),
    });
  }
  if (conflicts.length) {
    const paths = conflicts.map((c) => String(c.path ?? c.original ?? c));
    out.push({ kind: "conflict", paths, signature: sig(paths) });
  }
  // The live answer, then the remembered one — and the fallback covers a LIVE NULL as well as a
  // missing reply, because `last_sync_epoch_secs` does not survive a daemon restart:
  // `ControlShared::new` starts it at `None` and only a successful pass sets it. A daemon that
  // restarts and then cannot sync — an expired session, a full disk, exactly what this trigger is
  // for — would otherwise report `null` for ever and never cross a threshold at all.
  //
  // A machine that has genuinely never synced has no remembered value either, so it stays `null`
  // and produces no outage: that state is onboarding's, and `firstRun` is what draws it.
  const lastSync = response?.last_sync_epoch_secs ?? lastSeenSync;
  // A DELIBERATE PAUSE IS NOT AN OUTAGE. `pause and resume` is one of the twelve categories that
  // stay silent on purpose, and a folder paused over a weekend crosses the day threshold on its own.
  const paused = daemonState === "paused" || response?.paused === true;
  if (!paused && lastSync != null && nowSecs - lastSync >= OUTAGE_AFTER_SECS) {
    out.push({
      kind: "outage",
      changes: response?.pending_changes ?? null,
      // WHICH SENTENCE THE BODY GETS. `11a Outage` draws the expired-session one, and
      // `11-notifications.md` gives the trigger three causes — "an outage, expired session, or full
      // disk". Saying "Proton Drive is asking you to sign in again" about a full disk would be a
      // false statement in the banner whose job is to be trusted, so the other two causes take the
      // deck's own unreachable sentence instead. Both open with the reassurance.
      cause: daemonState === "authExpired" ? "auth" : "unreachable",
      // The episode, so a fresh outage after a good pass is a new thing to say and the same one is
      // not repeated every two seconds for a day.
      signature: `outage:${lastSync}`,
    });
  }
  if (lastSync != null) {
    out.push({ kind: "firstSync", signature: "first" });
  }
  return out.sort((a, b) => SEVERITY[b.kind] - SEVERITY[a.kind]);
}

/** The folder a view is kept under: `null` for the default folder, else its name. */
const homeOf = (view) => (view?.isDefault === false ? view.pair : null);

/** What is known about the folder kept as `home`: the top of the state for the default folder, else `seen`. */
function witnessOf(state, home) {
  if (home == null) return { sawUnsynced: state.sawUnsynced, lastSeenSync: state.lastSeenSync };
  return state.seen?.[seenKey(home)] ?? UNSEEN;
}

function setWitness(next, home, witness) {
  if (home == null) {
    next.sawUnsynced = witness.sawUnsynced;
    next.lastSeenSync = witness.lastSeenSync;
  } else {
    next.seen[seenKey(home)] = witness;
  }
}

/** The kind half of a `said` key, and the folder half (`null` for the default folder). */
function splitKey(key) {
  const at = key.indexOf("@");
  return at < 0 ? { kind: key, home: null } : { kind: key.slice(0, at), home: key.slice(at + 1) };
}

/**
 * The default folder changed from `next.owner` to `to`: move each folder's memory to where that folder now lives.
 *
 * The bare keys were the old default's. If it is still listed (`known`) it keeps them as an ordinary folder's;
 * if it is not, they are dropped — handed to `to` they would silence a conflict `to` never announced, because
 * a signature is only relative paths. `to`'s own memory, kept under `kind@to`, becomes the bare keys. The banner
 * on screen follows its folder, and is withdrawn (the return value) when its folder is the one that is gone.
 * Mutates `next`, which is `decide`'s own copy.
 */
function rehomeDefault(next, to, known) {
  const from = next.owner;
  const oldSeen = { sawUnsynced: next.sawUnsynced, lastSeenSync: next.lastSeenSync };
  const moved = {};
  for (const key of Object.keys(next.said)) {
    const { kind, home } = splitKey(key);
    if (home == null) {
      if (known.has(from)) moved[keyOf(kind, from)] = next.said[key];
    } else if (home === to) {
      moved[keyOf(kind, null)] = next.said[key];
    } else {
      moved[key] = next.said[key];
    }
  }
  next.said = moved;
  const incoming = next.seen[seenKey(to)] ?? UNSEEN;
  delete next.seen[seenKey(to)];
  if (known.has(from)) next.seen[seenKey(from)] = oldSeen;
  next.sawUnsynced = incoming.sawUnsynced;
  next.lastSeenSync = incoming.lastSeenSync;
  next.owner = to;

  if (!next.lastKind) return false;
  if (next.lastPair == null) {
    if (known.has(from)) {
      next.lastPair = from;
      return false;
    }
    return true;
  }
  if (next.lastPair === to) next.lastPair = null;
  return false;
}

/**
 * Decide what to show, if anything, and what to remember.
 *
 * Returns `{ event, state, resolved }` — `event` is null when nothing should interrupt, `resolved`
 * asks for the live banner to be taken down because its subject is gone, and `state` is always the
 * state to keep (it advances `sawUnsynced` and `lastSeenSync` even on a silent tick).
 *
 * `views` is one entry per folder the window can see: the world as `candidates` reads it, plus
 * `pair` (the folder's name, or `null` at one folder), `isDefault` and `unknown` (the kinds, `deletion` and/or
 * `conflict`, whose data has not landed for that folder — absent means all are known). Without it, `view` is
 * the one and only folder and everything is exactly what it was before folders existed. `roster` is the names
 * the daemon runs when it has just said so, or `null` when it has not; a folder it no longer lists is
 * forgotten, and its first entry is the default folder, whose name is kept as `state.owner`.
 *
 * THE FOLDERS ARE DECIDED TOGETHER, in one severity order, against ONE rate limit. Each folder's
 * triggers are its own (a signature is remembered per folder, so the same queue in two folders is two
 * things to say), but the 30-second window and "more serious jumps it" are about the BANNER on screen:
 * a deletion in `photos` jumps a conflict banner for `documents`, and two conflicts in two folders are
 * one banner now and the second one later, never two at once.
 */
export function decide({ state, view, views, roster = null, policy = "only_when_needed", nowMs }) {
  const nowSecs = Math.floor(nowMs / 1000);
  const allowed = ALLOWED[policy] ?? ALLOWED.only_when_needed;
  const seen = views ?? [{ ...(view ?? {}), pair: null, isDefault: true }];
  const next = { ...state, said: { ...state.said }, seen: { ...state.seen } };

  // A folder the daemon no longer runs is forgotten: what was said about it and whether it was watched,
  // so a folder added later under the same name announces its own first sync. The banner on screen, if
  // it was about that folder, is withdrawn: its buttons would act on a folder that is not there.
  let gone = false;
  if (roster) {
    const known = new Set(roster);
    // The first folder the daemon lists is the default one, as `notifierViews` reads it. A state with no
    // owner is read as the current default's own; one whose owner is another folder has had its default
    // changed under it, and its bare keys are not this folder's to inherit.
    if (roster.length) {
      if (next.owner == null) next.owner = roster[0];
      else if (next.owner !== roster[0]) gone = rehomeDefault(next, roster[0], known);
    }
    for (const key of Object.keys(next.said)) {
      const { home } = splitKey(key);
      if (home != null && !known.has(home)) delete next.said[key];
    }
    for (const key of Object.keys(next.seen)) {
      if (!known.has(key.slice(1))) delete next.seen[key];
    }
    gone = gone || (Boolean(next.lastKind) && next.lastPair != null && !known.has(next.lastPair));
    if (gone) {
      next.lastKind = null;
      next.lastPair = null;
    }
  }

  const entries = [];
  const viewed = new Set();
  // For each folder this tick could see: the kinds it could NOT hear (`view.unknown`), whose data has not
  // landed. Such a kind is neither said nor forgotten — see the forgetting rule below.
  const unheard = new Map();
  for (const folder of seen) {
    const home = homeOf(folder);
    viewed.add(home);
    unheard.set(home, new Set(folder?.unknown ?? []));
    const lastSync = folder?.response?.last_sync_epoch_secs ?? null;
    // From `next`, not `state`: a default folder that changed this tick has had its witnesses moved.
    const was = witnessOf(next, home);
    // Witnessed before anything is decided, so a tick that shows nothing still records what it saw.
    // `response` present and `last_sync` absent is the daemon answering "nothing has ever synced";
    // an unreachable daemon answers nothing at all and must not count as a witness.
    setWitness(next, home, {
      sawUnsynced:
        was.sawUnsynced || (Boolean(folder?.response) && lastSync == null && was.lastSeenSync == null),
      lastSeenSync: lastSync ?? was.lastSeenSync ?? null,
    });
    for (const candidate of candidates({ ...(folder ?? {}), lastSeenSync: was.lastSeenSync }, nowSecs)) {
      entries.push({ event: folder?.pair == null ? candidate : { ...candidate, pair: folder.pair }, home });
    }
  }
  // Stable, so equal severities stay in the order the folders were given.
  entries.sort((a, b) => SEVERITY[b.event.kind] - SEVERITY[a.event.kind]);
  const present = new Set(entries.map(({ event, home }) => keyOf(event.kind, home)));

  // FORGETTING IS PART OF THE RULE. What we said about a kind is remembered so the same queue does
  // not repeat; once that queue is empty there is nothing left to repeat, and holding the signature
  // would silence the identical set if it ever came back — a conflict resolved on Monday and made
  // again on Tuesday is a new thing to say. `firstSync` is the exception, because "once, ever" is
  // its whole specification.
  //
  // ONLY FOR A FOLDER THIS TICK COULD SEE. A folder the window has no live reading of (the daemon is
  // not answering) has not been shown to have an empty queue, and forgetting what was said about it
  // would say it all again when the daemon came back. At one folder that folder is always seen, so
  // this is the rule it always was.
  //
  // AND ONLY FOR A KIND WHOSE DATA HAS LANDED. A folder that is not on screen is summarised every poll but
  // its queue and its conflicts are separate reads, and on the first poll after a launch neither has come
  // home: an empty list there is "not yet", not "nothing". Forgetting on it said every standing banner again
  // once per launch. Until a kind's data has landed the folder is not seen for that kind, so it neither says
  // nor forgets anything about it.
  for (const key of Object.keys(next.said)) {
    const { kind, home } = splitKey(key);
    if (kind !== "firstSync" && viewed.has(home) && !unheard.get(home).has(kind) && !present.has(key)) {
      delete next.said[key];
    }
  }

  // The live banner is about something that no longer exists — approved, resolved, or synced. It
  // comes down rather than sitting there as a question nobody can answer any more. Of a folder this
  // tick could not see it says nothing: a banner is not withdrawn on no evidence.
  const liveHome = next.lastPair ?? null;
  const liveKind = gone ? null : next.lastKind;
  const resolved =
    gone ||
    (Boolean(liveKind) &&
      liveKind !== "firstSync" &&
      viewed.has(liveHome) &&
      !unheard.get(liveHome).has(liveKind) &&
      !present.has(keyOf(liveKind, liveHome)));
  if (resolved) {
    next.lastKind = null;
    next.lastPair = null;
  }

  for (const { event, home } of entries) {
    if (!allowed.has(event.kind)) continue;
    // Fires once, ever, per folder, and only where this install watched the wait end.
    if (event.kind === "firstSync" && !witnessOf(next, home).sawUnsynced) continue;
    const key = keyOf(event.kind, home);
    if (next.said[key] === event.signature) continue;

    // The rate limit, and the one thing that may jump it: something more serious than what is
    // already on screen. Waiting 25 seconds to say that files are about to be deleted, because a
    // conflict banner went up five seconds ago, is the wrong way round.
    // `> 0` as well as `< COALESCE_MS`: a clock that steps backwards (or a state written on another
    // machine) leaves `lastAt` in the future, and an unclamped comparison would then silence every
    // banner until the wall clock caught up — hours, on a timezone-sized step.
    const since = nowMs - state.lastAt;
    const withinWindow = since >= 0 && since < COALESCE_MS;
    const moreSerious = SEVERITY[event.kind] > (SEVERITY[state.lastKind] ?? -1);
    if (withinWindow && !moreSerious) return { event: null, state: next, resolved };

    next.said[key] = event.signature;
    next.lastAt = nowMs;
    next.lastKind = event.kind;
    next.lastPair = home;
    // A banner that replaces the one that resolved does not also need it taken down.
    return { event, state: next, resolved: false };
  }
  return { event: null, state: next, resolved };
}

/**
 * What the window can see of every folder, as `decide` reads it — and the names the daemon runs when it
 * has just said so. `select` is the store's.
 *
 * TWO ADDRESSES FOR THE SAME FACTS, ONE BUILDER. The folder on screen is read from the store's selected
 * slice, exactly as it always was; every other folder is read from the roster's summary plus what the
 * poll has fetched for it (`refreshOtherPairs`): its withheld deletions while its summary says it has
 * any (the summary carries a COUNT, and "permanent" needs each item's direction), and its conflicts from
 * the scan that runs once a minute. Nothing here asks the daemon for anything. No second schedule.
 *
 * THE LIVE ROSTER AND NOTHING ELSE. The store keeps the last roster across a failed read, because that
 * is how the window still NAMES its folders, and it is the wrong thing to decide from: a stopped daemon
 * would go on being read as "photos still has a deletion waiting" and "photos last synced yesterday",
 * and a banner built from that is #246's false statement said aloud, about a folder nothing has looked
 * at since. `rosterLive` is the store's one answer (`livePairs`/`livePairStates` apply it). With the daemon
 * silent only the folder on screen is read, and from its own slice, as at one folder.
 *
 * `pair` is `null` below two folders — the banner names a folder only when there is a choice of them.
 * `unknown` names the kinds of a folder that is not on screen whose data has not landed (above); the folder on
 * screen, and the one folder below two, have none — their lists arrive with the reply the window is drawn from.
 */
export function notifierViews(select) {
  const shown = {
    response: select.response(),
    conflicts: select.conflicts(),
    daemonState: select.daemonState(),
  };
  const named = select.pairs();
  const roster = select.rosterLive() ? named.map((summary) => summary.name) : null;
  if (named.length < 2) return { views: [{ ...shown, pair: null, isDefault: true }], roster };

  // The default folder is the first the daemon lists — the one an unaddressed request means.
  const defaultName = named[0].name;
  const selected = select.pairName();
  const folder = (name, parts) => ({ ...parts, pair: name, isDefault: name === defaultName });
  const states = new Map(select.livePairStates().map((entry) => [entry.name, entry.state]));

  const views = select.livePairs().map((summary) => {
    if (summary.name === selected) return folder(summary.name, shown);
    // WHAT HAS NOT LANDED IS NOT THERE. The summary is as fresh as the poll; the list behind its count and the
    // conflict scan are separate reads that come home later, and a list fetched BEFORE the count last changed
    // is the old queue's — the one `Keep them` drained, which nothing refetches at 0. A kind whose data is not
    // known to be about the folder as it is now is listed in `unknown`, and `decide` neither says nor forgets
    // anything about it (`11-notifications.md`: "Only what is known is said"). A count of 0 needs no list.
    const unknown = [];
    const queued = summary.pending_deletions > 0;
    const queue = queued && select.deletionsFreshOf(summary.name);
    if (queued && !queue) unknown.push("deletion");
    const scanned = select.conflictsFreshOf(summary.name);
    if (!scanned) unknown.push("conflict");
    return folder(summary.name, {
      response: {
        pending_changes: summary.pending_changes,
        last_sync_epoch_secs: summary.last_sync_epoch_secs,
        paused: summary.paused,
        // Its queue is the one the poll fetched, and only while the summary says there is one: a list
        // fetched earlier and not since is not evidence about a queue the summary now counts as empty.
        pending_deletions: queue ? select.pendingDeletionsOf(summary.name) : [],
      },
      conflicts: scanned ? select.conflictsOf(summary.name) : [],
      daemonState: states.get(summary.name) ?? null,
      unknown,
    });
  });
  if (!views.some((entry) => entry.pair === selected)) views.unshift(folder(selected, shown));
  return { views, roster };
}
