// The state matrix (F3): fixed copy + actions + counter behaviour per daemon state (design §6).
// One computation, so the headline/sub-line/buttons/banner never disagree across the window.
// `mono` = the daemon status string shown verbatim in the state pill.

export const STATE_MATRIX = {
  running: {
    headline: "Syncing…",
    subline: "reconciling changes",
    actions: [
      { label: "Sync now", cmd: "syncNow", kind: "primary", hint: "syncnow" },
      { label: "Pause", cmd: "pause", kind: "secondary", hint: "pause" },
    ],
    showTransfers: true,
    countersUnknown: false,
    pillMono: "running",
  },
  idle: {
    headline: "Everything is up to date",
    subline: "last synced",
    actions: [
      { label: "Sync now", cmd: "syncNow", kind: "primary", hint: "syncnow" },
      { label: "Pause", cmd: "pause", kind: "secondary", hint: "pause" },
    ],
    showTransfers: false,
    countersUnknown: false,
    pillMono: "idle",
  },
  paused: {
    headline: "Syncing is paused",
    subline: "edits are still tracked",
    actions: [{ label: "Resume", cmd: "resume", kind: "primary", hint: "resume" }],
    showTransfers: false,
    countersUnknown: false,
    pillMono: "paused",
  },
  authExpired: {
    headline: "Proton sign-in expired",
    subline: "re-authenticate the proton-drive CLI",
    actions: [
      { label: "Re-authenticate", cmd: "reauth", kind: "primary", hint: "proton-drive login" },
      { label: "Pause", cmd: "pause", kind: "secondary", hint: "pause" },
    ],
    showTransfers: false,
    countersUnknown: false,
    pillMono: "auth expired",
  },
  unreachable: {
    headline: "Can't reach the sync daemon",
    subline: "your files are untouched",
    actions: [
      {
        label: "Start proton-syncd",
        cmd: "startService",
        kind: "primary",
        hint: "systemctl --user start proton-syncd",
      },
      { label: "View journal", cmd: "journal", kind: "secondary", hint: "journalctl --user -u proton-syncd" },
    ],
    showTransfers: false,
    countersUnknown: true,
    pillMono: "unreachable",
  },
  firstRun: {
    headline: "Nothing has synced yet",
    subline: "review a plan before the first pass",
    actions: [
      { label: "Preview plan", cmd: "previewPlan", kind: "primary", hint: "proton-syncd --dry-run" },
      { label: "Choose folders", cmd: "chooseFolders", kind: "secondary", hint: "settings" },
    ],
    showTransfers: false,
    countersUnknown: true,
    pillMono: "first run",
  },
};

export function matrixFor(daemonState) {
  return STATE_MATRIX[daemonState] ?? STATE_MATRIX.unreachable;
}

// Onboarding routing (S8). The onboarding takeover is a *latched* decision, not a raw read of the
// daemon state, because it must survive the mid-flow config write: the moment step 2 writes the
// folder pair, a naive `!hasPair` entry condition would flip false and the next 2 s poll would eject
// the user to the unreachable screen. So: once we enter onboarding we STAY until the daemon becomes
// *reachable* — at which point the main screen (with its per-state actions) takes over.
//
// Release on ANY reachable state — `idle`/`running`/`paused` (setup succeeded) AND `authExpired`
// (setup succeeded but Proton sign-in lapsed): onboarding can't fix expired auth, and its step-4
// status line already promises a hand-off once the state moves past `firstRun`, so we must actually
// hand off to the main screen's Re-authenticate action rather than trap the user in the wizard.
//
// Entry has two triggers: (1) `firstRun` — the canonical signal (a reachable daemon that has never
// synced), preserving the original single-hook behaviour; and (2) a genuinely fresh machine — a
// *completed* status poll reports the daemon unreachable AND no folder pair is configured yet.
// `statusPolled` gates this so we never claim "fresh" from the pre-poll default state, and
// `configLoaded` gates it so a daemon configured elsewhere (or a config file not yet read) doesn't
// flash the wizard. Pure and side-effect free so it can be unit-tested.
export function nextOnboardingLatch(prev, daemonState, hasConfigPair, configLoaded, statusPolled) {
  // Reachable in any form — hand back to the main screen (which surfaces the right per-state action).
  if (
    daemonState === "idle" ||
    daemonState === "running" ||
    daemonState === "paused" ||
    daemonState === "authExpired"
  )
    return false;
  // Reachable daemon that has never synced: the original firstRun takeover.
  if (daemonState === "firstRun") return true;
  // Fresh machine: a completed poll says the daemon is unreachable AND no folders are chosen.
  if (daemonState === "unreachable" && statusPolled && configLoaded && !hasConfigPair) return true;
  // Anything else (notably `unreachable` right after step 2 wrote the config, or before the first
  // poll): hold whatever we were doing so the flow isn't interrupted.
  return prev;
}
