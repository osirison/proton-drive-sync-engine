// Data adapter (F1). Every screen talks to the backend through this module, never `window.__TAURI__`
// directly — so the same frontend runs inside Tauri (real daemon) and in a plain browser (mock data
// for design preview). The command names here are the fixed surface defined in gui/src-tauri.

import { activeFixture } from "./fixtures/frames.js";

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
  // `literalPath: true` (the default) marks `target` as a row's actual relative path, so a file
  // literally named "all" can never be mistaken for the every-item selector; the Approve-all /
  // Deny-all buttons pass `false` with the explicit "all" argument.
  approve: (target, literalPath = true) => invoke("approve", { target, literalPath }),
  deny: (target, literalPath = true) => invoke("deny", { target, literalPath }),
  listPendingDeletions: () => invoke("list_pending_deletions"),
  readConfig: () => invoke("read_config"),
  writeConfig: (update) => invoke("write_config", { update }),
  runDryRun: () => invoke("run_dry_run"),
  listRemote: (path) => invoke("list_remote", { path: path ?? null }),
  scanConflicts: () => invoke("scan_conflicts"),
  resolveConflict: (conflict, choice) => invoke("resolve_conflict", { conflict, choice }),
  readConflictPair: (conflict) => invoke("read_conflict_pair", { conflict }),
  pathSyncStatus: (relativePath) => invoke("path_sync_status", { relativePath }),
  startService: () => invoke("start_service"),
  restartService: () => invoke("restart_service"),
  notify: (title, body) => invoke("notify", { title, body }),
  // F4's Ctrl W / Ctrl Q. Both go through the same backend paths the tray menu uses, so the
  // shortcut and the menu item cannot drift apart. Note quitting does NOT stop the daemon — see
  // the comment on `quit_app` in commands.rs.
  closeWindow: () => invoke("close_window"),
  quitApp: () => invoke("quit_app"),
  // Subscribe to the backend's `tray-navigate` event (tray menu → tab switch). Routed through the
  // facade so screens/shell never touch `window.__TAURI__` directly; a no-op in browser preview.
  onTrayNavigate: (cb) => {
    if (!inTauri()) return;
    // `listen` returns a Promise (resolving to an unlisten fn); handle rejection so a failed
    // registration surfaces instead of becoming a silent unhandled rejection. We don't need the
    // unlisten handle — the listener lives for the app's lifetime.
    window.__TAURI__.event
      .listen("tray-navigate", (e) => cb(e.payload))
      .catch((err) => console.error("tray-navigate listen failed:", err));
  },
  isMock: () => !inTauri(),
};

// ---- browser-preview mock (never runs inside Tauri) ----
// `?frame=<label>` swaps the generic mock for that frame's fixture (F9), so the same dataset drives
// the fidelity harness and the design preview. Without a frame the generic mock below still runs,
// which is what keeps the browser preview useful before every frame has a fixture.
function mockInvoke(cmd, args) {
  const fixture = activeFixture();
  if (fixture) {
    // ONE COMMAND PER LINE, and a fixture key only where the reply is not already inside the status.
    // `list_pending_deletions` reads through to `status.response.pending_deletions` because that is
    // literally what commands.rs does — it sends a plain `Status` and returns that field — so a
    // fixture carrying both cannot be made to disagree with a real daemon. `read_config` falls back
    // the same way: a daemon that is up reports its own roots, and its reply outranks the file.
    //
    // A command a fixture says nothing about falls through to the generic mock below, which is what
    // keeps a partly-described frame useful rather than blank.
    switch (cmd) {
      case "get_status":
        return Promise.resolve(fixture.status);
      case "scan_conflicts":
        return Promise.resolve(fixture.conflicts ?? []);
      case "list_pending_deletions":
        return Promise.resolve(fixture.deletions ?? fixture.status?.response?.pending_deletions ?? []);
      case "read_config":
        // NO FALLBACK TO `status.response.config`, and the near-miss is the point: both are called
        // `config` and both carry `local_root`/`remote_root`, but they are different types answering
        // different questions. `read_config` returns `ConfigPayload` — what the TOML file says, with
        // `toml`, `exists`, `include`/`exclude`, `scan_interval_secs` and the rest. A status reply's
        // `config` is `RunningConfigInfo`: three paths describing the process that is actually
        // running. The old fallback handed the file's shape to a screen and filled it with the
        // daemon's, which reads correctly on the two shared keys and is missing every other one —
        // so Settings would have drawn an empty skip list rather than an unanswered one.
        //
        // An empty doc is the honest answer for a frame that describes no config file, and it is a
        // real state the app already handles: `refreshConfig` treats a missing file as `{}` rather
        // than an error. The 38 frames without one lose nothing — the footer's folder pair reads the
        // STATUS first (`app.js`'s `live?.local_root ?? configInfo?.local_root`), which is the
        // correct precedence anyway: a running daemon's roots are ground truth and the file is the
        // fallback.
        return Promise.resolve(fixture.config ?? {});
      case "read_conflict_pair":
        if (fixture.conflictPair) return Promise.resolve(fixture.conflictPair);
        break;
      case "run_dry_run":
        if (fixture.dryRun) return Promise.resolve(fixture.dryRun);
        break;
      case "path_sync_status":
        // Keyed by the path asked for. An unlisted path answers `tracked: false` rather than falling
        // through to the generic mock: "this frame does not describe that file" is a real answer, and
        // it is the one a never-synced file gets from the real command.
        if (fixture.pathStatus)
          return Promise.resolve(fixture.pathStatus[args?.relativePath] ?? { tracked: false });
        break;
      default:
        break;
    }
  }
  switch (cmd) {
    case "get_status":
      return Promise.resolve({
        state: "running",
        response: {
          status: "syncing",
          paused: false,
          syncing: true,
          reconcile_seq: 7,
          pending_changes: 3,
          message: "sync completed",
          last_sync_epoch_secs: Math.floor(Date.now() / 1000) - 120,
          last_error: null,
          last_plan_summary: {
            total: 5,
            uploads: 2,
            downloads: 1,
            remote_directories_created: 0,
            local_directories_created: 0,
            local_moves: 0,
            remote_moves: 0,
            auto_links: 0,
            conflicts: 1,
            type_conflicts: 0,
            remote_deletes: 0,
            local_deletes: 0,
            purges: 0,
            skipped_unsupported: 1,
            destructive_actions: 0,
          },
          last_successful_sync_summary: null,
          status_history: [
            {
              epoch_secs: Math.floor(Date.now() / 1000) - 120,
              message: "sync completed",
              last_error: null,
              plan_summary: null,
              successful_sync_summary: null,
            },
            {
              epoch_secs: Math.floor(Date.now() / 1000) - 900,
              message: "uploaded 2 files",
              last_error: null,
              plan_summary: null,
              successful_sync_summary: null,
            },
          ],
          pending_deletions: [],
          config: {
            local_root: "~/ProtonDrive",
            remote_root: "/Drive/RemoteFolder",
            db_path: "~/ProtonDrive/.sync/sync_index.db",
          },
        },
      });
    case "start_service":
      return Promise.resolve("asked systemd to start proton-syncd (preview mock)");
    case "approve":
    case "deny":
      // Simulate the daemon round trip so the Deletions screen's busy → acknowledged flow is
      // visible in browser preview. Shaped like a real StatusPayload: the screen only trusts a
      // reply that carries a `response` and no `error`.
      return new Promise((resolve) => {
        setTimeout(() => resolve({ state: "running", response: { paused: false }, error: null }), 800);
      });
    case "restart_service":
      // Simulate the real stop→start latency so the Settings screen's "Restarting…" state is
      // visible in browser preview.
      return new Promise((resolve) => {
        setTimeout(() => resolve("daemon restarted (preview mock)"), 1200);
      });
    case "pause":
      return Promise.resolve({
        state: "paused",
        response: {
          status: "paused",
          paused: true,
          pending_changes: 3,
          message: "paused",
          last_sync_epoch_secs: null,
          last_error: null,
          last_plan_summary: null,
          last_successful_sync_summary: null,
          status_history: [],
          pending_deletions: [],
        },
      });
    case "scan_conflicts":
      return Promise.resolve([{ original: "notes/todo.txt", sidecar: "notes/todo.proton-cloud.txt" }]);
    case "read_conflict_pair":
      return Promise.resolve({
        original: {
          exists: true,
          size: 41,
          mtime_epoch_secs: Math.floor(Date.now() / 1000) - 300,
          text: "# Todo\n- buy milk\n- call Alice\n- ship v1\n",
          binary_or_large: false,
        },
        sidecar: {
          exists: true,
          size: 44,
          mtime_epoch_secs: Math.floor(Date.now() / 1000) - 120,
          text: "# Todo\n- buy oat milk\n- call Alice\n- ship v1\n- relax\n",
          binary_or_large: false,
        },
      });
    case "resolve_conflict":
      return Promise.resolve(null);
    case "list_pending_deletions":
      return Promise.resolve([]);
    case "read_config":
      return Promise.resolve({
        path: "~/.config/proton-sync/proton-sync.toml",
        exists: true,
        toml: "# preview\n",
        local_root: "~/ProtonDrive",
        remote_root: "/Drive/RemoteFolder",
        scan_interval_secs: 300,
        events_driven: true,
        include: [],
        exclude: ["*.tmp"],
        proton_cli: "proton-drive",
        proton_timeout_secs: 60,
        proton_list_attempts: 3,
        delete_approval_remote: true,
        delete_approval_local: true,
      });
    default:
      return Promise.resolve(null);
  }
}
