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

import { severityOf } from "./ui/rows.js";

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

/** The state that has to survive a restart. Serialisable on purpose — `app.js` keeps it in storage. */
export const emptyState = () => ({
  /** kind → the signature of the last thing we said about it. */
  said: {},
  /** When we last showed anything, in ms, and about what. */
  lastAt: 0,
  lastKind: null,
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
 * Permanent = the file leaves this computer for good.
 *
 * `severityOf` RATHER THAN A SECOND TEST, and its default is why: it answers `permanent` for
 * anything that is not the literal `remote`, so a direction nobody anticipated warns rather than
 * stays silent. A `direction === "local"` here would be the same rule written the fail-open way,
 * in the one trigger where being wrong costs files. S3 made this call for the screen (DEVIATIONS
 * §76); one implementation, not two that can drift.
 */
const isPermanent = (deletion) => severityOf(deletion?.direction) === "permanent";

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

/**
 * Decide what to show, if anything, and what to remember.
 *
 * Returns `{ event, state, resolved }` — `event` is null when nothing should interrupt, `resolved`
 * asks for the live banner to be taken down because its subject is gone, and `state` is always the
 * state to keep (it advances `sawUnsynced` and `lastSeenSync` even on a silent tick).
 */
export function decide({ state, view, policy = "only_when_needed", nowMs }) {
  const nowSecs = Math.floor(nowMs / 1000);
  const allowed = ALLOWED[policy] ?? ALLOWED.only_when_needed;
  const lastSync = view?.response?.last_sync_epoch_secs ?? null;
  // Witnessed before anything is decided, so a tick that shows nothing still records what it saw.
  // `response` present and `last_sync` absent is the daemon answering "nothing has ever synced";
  // an unreachable daemon answers nothing at all and must not count as a witness.
  const next = {
    ...state,
    said: { ...state.said },
    sawUnsynced:
      state.sawUnsynced || (Boolean(view?.response) && lastSync == null && state.lastSeenSync == null),
    lastSeenSync: lastSync ?? state.lastSeenSync ?? null,
  };

  const list = candidates({ ...(view ?? {}), lastSeenSync: state.lastSeenSync }, nowSecs);
  const present = new Set(list.map((c) => c.kind));

  // FORGETTING IS PART OF THE RULE. What we said about a kind is remembered so the same queue does
  // not repeat; once that queue is empty there is nothing left to repeat, and holding the signature
  // would silence the identical set if it ever came back — a conflict resolved on Monday and made
  // again on Tuesday is a new thing to say. `firstSync` is the exception, because "once, ever" is
  // its whole specification.
  for (const kind of Object.keys(next.said)) {
    if (kind !== "firstSync" && !present.has(kind)) delete next.said[kind];
  }

  // The live banner is about something that no longer exists — approved, resolved, or synced. It
  // comes down rather than sitting there as a question nobody can answer any more.
  const resolved = Boolean(state.lastKind) && state.lastKind !== "firstSync" && !present.has(state.lastKind);
  if (resolved) next.lastKind = null;

  for (const candidate of list) {
    if (!allowed.has(candidate.kind)) continue;
    // Fires once, ever, and only where this install watched the wait end.
    if (candidate.kind === "firstSync" && !next.sawUnsynced) continue;
    if (next.said[candidate.kind] === candidate.signature) continue;

    // The rate limit, and the one thing that may jump it: something more serious than what is
    // already on screen. Waiting 25 seconds to say that files are about to be deleted, because a
    // conflict banner went up five seconds ago, is the wrong way round.
    // `> 0` as well as `< COALESCE_MS`: a clock that steps backwards (or a state written on another
    // machine) leaves `lastAt` in the future, and an unclamped comparison would then silence every
    // banner until the wall clock caught up — hours, on a timezone-sized step.
    const since = nowMs - state.lastAt;
    const withinWindow = since >= 0 && since < COALESCE_MS;
    const moreSerious = SEVERITY[candidate.kind] > (SEVERITY[state.lastKind] ?? -1);
    if (withinWindow && !moreSerious) return { event: null, state: next, resolved };

    next.said[candidate.kind] = candidate.signature;
    next.lastAt = nowMs;
    next.lastKind = candidate.kind;
    // A banner that replaces the one that resolved does not also need it taken down.
    return { event: candidate, state: next, resolved: false };
  }
  return { event: null, state: next, resolved };
}
