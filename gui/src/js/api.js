// Data adapter (F1). Every screen talks to the backend through this module, never `window.__TAURI__`
// directly — so the same frontend runs inside Tauri (real daemon) and in a plain browser (mock data
// for design preview). The command names here are the fixed surface defined in gui/src-tauri.

const inTauri = () => typeof window !== "undefined" && !!window.__TAURI__;

export async function invoke(cmd, args) {
  if (inTauri()) {
    return window.__TAURI__.core.invoke(cmd, args);
  }
  return mockInvoke(cmd, args);
}

// Thin named wrappers over the fixed command surface.
export const api = {
  getStatus: () => invoke("get_status"),
  pause: () => invoke("pause"),
  resume: () => invoke("resume"),
  syncNow: () => invoke("sync_now"),
  approve: (target) => invoke("approve", { target }),
  deny: (target) => invoke("deny", { target }),
  listPendingDeletions: () => invoke("list_pending_deletions"),
  readConfig: () => invoke("read_config"),
  writeConfig: (update) => invoke("write_config", { update }),
  runDryRun: () => invoke("run_dry_run"),
  listRemote: (path) => invoke("list_remote", { path: path ?? null }),
  scanConflicts: () => invoke("scan_conflicts"),
  resolveConflict: (conflict, choice) => invoke("resolve_conflict", { conflict, choice }),
  readConflictPair: (conflict) => invoke("read_conflict_pair", { conflict }),
  pathSyncStatus: (relativePath) => invoke("path_sync_status", { relativePath }),
  notify: (title, body) => invoke("notify", { title, body }),
  isMock: () => !inTauri(),
};

// ---- browser-preview mock (never runs inside Tauri) ----
function mockInvoke(cmd, _args) {
  switch (cmd) {
    case "get_status":
      return Promise.resolve({
        state: "running",
        response: {
          status: "running",
          paused: false,
          pending_changes: 3,
          message: "sync completed",
          last_sync_epoch_secs: Math.floor(Date.now() / 1000) - 120,
          last_error: null,
          last_plan_summary: {
            total: 5, uploads: 2, downloads: 1, remote_directories_created: 0,
            local_directories_created: 0, local_moves: 0, remote_moves: 0, auto_links: 0,
            conflicts: 1, type_conflicts: 0, remote_deletes: 0, local_deletes: 0,
            purges: 0, skipped_unsupported: 1, destructive_actions: 0,
          },
          last_successful_sync_summary: null,
          status_history: [
            { epoch_secs: Math.floor(Date.now() / 1000) - 120, message: "sync completed", last_error: null, plan_summary: null, successful_sync_summary: null },
            { epoch_secs: Math.floor(Date.now() / 1000) - 900, message: "uploaded 2 files", last_error: null, plan_summary: null, successful_sync_summary: null },
          ],
          pending_deletions: [],
        },
      });
    case "pause":
      return Promise.resolve({ state: "paused", response: { status: "paused", paused: true, pending_changes: 3, message: "paused", last_sync_epoch_secs: null, last_error: null, last_plan_summary: null, last_successful_sync_summary: null, status_history: [], pending_deletions: [] } });
    case "scan_conflicts":
      return Promise.resolve([{ original: "notes/todo.txt", sidecar: "notes/todo.proton-cloud.txt" }]);
    case "read_conflict_pair":
      return Promise.resolve({
        original: { exists: true, size: 41, mtime_epoch_secs: Math.floor(Date.now() / 1000) - 300, text: "# Todo\n- buy milk\n- call Alice\n- ship v1\n", binary_or_large: false },
        sidecar: { exists: true, size: 44, mtime_epoch_secs: Math.floor(Date.now() / 1000) - 120, text: "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n- relax\n", binary_or_large: false },
      });
    case "resolve_conflict":
      return Promise.resolve(null);
    case "list_pending_deletions":
      return Promise.resolve([]);
    case "read_config":
      return Promise.resolve({
        path: "~/.config/proton-sync/proton-sync.toml", exists: true, toml: "# preview\n",
        local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder",
        scan_interval_secs: 300, events_driven: true, include: [], exclude: ["*.tmp"],
        proton_cli: "proton-drive", proton_timeout_secs: 60, proton_list_attempts: 3,
        delete_approval_remote: true, delete_approval_local: true,
      });
    default:
      return Promise.resolve(null);
  }
}
