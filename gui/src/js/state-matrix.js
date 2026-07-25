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
    pillMono: "running",
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
      { label: "Start proton-syncd", cmd: "startService", kind: "primary", hint: "systemctl --user start proton-syncd" },
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
