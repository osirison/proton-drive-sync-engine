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
  // Settings › `Sweep now`. NOT `syncNow`: this latches the next pass to a full-tree walk, which is
  // the whole difference between the two under an event-driven default.
  resync: () => invoke("resync"),
  // `literalPath: true` (the default) marks `target` as a row's actual relative path, so a file
  // literally named "all" can never be mistaken for the every-item selector; the Approve-all /
  // Deny-all buttons pass `false` with the explicit "all" argument.
  // `direction` is only read when NOTHING PENDING matches `target` — the Plan screen approving its
  // own plan's deletion before any pass has withheld it (#227). A pending item's own direction wins,
  // and an approval with neither authorises nothing.
  approve: (target, literalPath = true, direction = null) =>
    invoke("approve", { target, literalPath, direction }),
  deny: (target, literalPath = true) => invoke("deny", { target, literalPath }),
  // `Keep it` / `Keep both files` (#224). NOT `deny`, which only revokes an approval: this refuses
  // the deletion — the daemon purges the baseline record and puts the surviving copy back on the
  // other side — so the row does not return on the next poll.
  keep: (target, literalPath = true) => invoke("keep", { target, literalPath }),
  listPendingDeletions: () => invoke("list_pending_deletions"),
  readConfig: () => invoke("read_config"),
  writeConfig: (update) => invoke("write_config", { update }),
  // Settings › `Choose…`. Resolves `null` when the picker is DISMISSED and rejects when it could
  // not open — the two were one answer until Copilot's second pass, which made a broken picker
  // indistinguishable from a closed one.
  chooseFolder: (start) => invoke("choose_folder", { start: start ?? null }),
  runDryRun: () => invoke("run_dry_run"),
  // `apply <token>` (#100). Resolves with the daemon's typed `ApplyOutcome` — `applied`,
  // `diverged`, `stale`, `paused`, `failed` — because what the screen does next depends on
  // which one, and a client must never tell them apart by matching a sentence (#103).
  applyPlan: (token, skipDestructive) =>
    invoke("apply_plan", { token, skipDestructive: skipDestructive === true }),
  // `listRemote` WAS HERE and is gone with the command behind it (#311): it shelled the
  // `proton-drive` CLI from the GUI process, beside the daemon's own client, for no caller at all.
  // A remote listing is `ControlCommand::List` over the socket when something needs one.
  scanConflicts: () => invoke("scan_conflicts"),
  resolveConflict: (conflict, choice) => invoke("resolve_conflict", { conflict, choice }),
  readConflictPair: (conflict) => invoke("read_conflict_pair", { conflict }),
  pathSyncStatus: (relativePath) => invoke("path_sync_status", { relativePath }),
  // The lookup field's search (S5). Takes a name, a path fragment or a pasted absolute path and
  // answers `{ matches: [{ path, status }], total, query }` — the resolved query included, because
  // the backend is what expands `~` and strips the sync root, so the screen must not re-derive it.
  searchFiles: (query, limit) => invoke("search_files", { query, limit: limit ?? null }),
  startService: () => invoke("start_service"),
  // Resolves with the typed `RestartOutcome` — `{ ending, detail|reason }` — because the five
  // endings are five different things to say about someone's files and two of them are opposites:
  // after `not_started` nothing is running, after `never_stopped` the OLD daemon still is (#335).
  // A Tauri `Err` crosses this bridge as a bare string, so an ending carried there would be prose
  // the screen matched on, which is #103's bug; the rejection is now infrastructure only.
  // `onlyIfRunning` is the save's own question (#320): a save must not start a daemon nobody asked
  // to start, and the probe belongs on the Rust side because the GUI's status is up to one poll
  // old. The retry after an unresolved restart passes `false` — it may have a stopped daemon to fix.
  restartService: (onlyIfRunning = false) => invoke("restart_service", { onlyIfRunning }),
  // The four openers (#220/#231). ALL OF THEM REJECT on failure — unlike the status commands, whose
  // error travels inside a resolved payload — so a caller that does not catch loses the only account
  // of why nothing opened. `openPaths` takes RELATIVE paths (both sides of a conflict); the backend
  // joins them onto the sync root and refuses anything that is not under it. `openRemote` takes no
  // argument at all: the URL is a constant in Rust, because no per-file Proton Drive link is
  // derivable from anything the daemon reports.
  openPaths: (relative) => invoke("open_paths", { relative }),
  openFolder: (relative) => invoke("open_folder", { relative }),
  openRemote: () => invoke("open_remote"),
  openSystemLog: () => invoke("open_system_log"),
  // The notification banners (S9). `payload` is `payloadFor(spec)` from `ui/notification.js`, so the
  // sentence a desktop shows and the one `renderBanner` draws come from the same builder.
  sendNotification: (payload) => invoke("send_notification", { payload }),
  closeNotification: () => invoke("close_notification"),
  // `notify_policy` (C6) — GUI-local, in the GUI's own `gui.toml`. It is never sent to the daemon:
  // "Never" must not change engine behaviour, and it cannot, because the daemon never sees it.
  readNotifyPolicy: () => invoke("read_notify_policy"),
  writeNotifyPolicy: (policy) => invoke("write_notify_policy", { policy }),
  // The Phase-1 capability commands (C2/C4/C5). `path` prices a folder before the config is
  // written; omitted, `free_space` uses the configured local root.
  freeSpace: (path) => invoke("free_space", { path: path ?? null }),
  checkCli: () => invoke("check_cli"),
  skipRuleUsage: (patterns, include) => invoke("skip_rule_usage", { patterns, include: include ?? null }),
  // F4's Ctrl W / Ctrl Q. Both go through the same backend paths the tray menu uses, so the
  // shortcut and the menu item cannot drift apart.
  //
  // SINCE S8, QUITTING STOPS THE DAEMON. It did not, and the sub-label `10-tray.md` requires beside
  // `Quit` says "stops syncing" — so the moment S8 drew that label, the app either had to keep the
  // promise or print a false one in the place the design says matters most. `Close window · keeps
  // syncing` is the other path and is unchanged. DEVIATIONS §45 (was open, now settled) and §82m.
  closeWindow: () => invoke("close_window"),
  quitApp: () => invoke("quit_app"),
  // The tray panel (S8). One id space with the native fallback menu — see `tray_action`.
  trayAction: (id) => invoke("tray_action", { id }),
  resizeTrayPanel: (height) => invoke("resize_tray_panel", { height }),
  hideTrayPanel: () => invoke("hide_tray_panel"),
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
  /**
   * A click on one of a banner's buttons (S9). The payload is `{ id, kind, action }` — the action id
   * from `SAFE_ACTIONS`, and which of the four events it belongs to.
   *
   * `notify.rs` has already checked the notification id against its own before emitting, because
   * `ActionInvoked` is broadcast to every listener on the bus.
   */
  onNotificationAction: (cb) => {
    if (!inTauri()) return;
    window.__TAURI__.event
      .listen("notification-action", (e) => cb(e.payload))
      .catch((err) => console.error("notification-action listen failed:", err));
  },
  isMock: () => !inTauri(),
};

/**
 * What `read_config` returns when the config file is not there — every field, as `commands.rs` fills
 * them in (`ConfigPayload`, lines 200-215): `exists: false`, an empty `toml` from an empty doc, and
 * `ConfigDoc`'s getters answering `None` for each scalar and an empty `Vec` for each array.
 *
 * Written out rather than abbreviated because the whole point is the shape. A missing file is not a
 * missing reply, and the mock must not be the only place that thinks otherwise.
 */
export const EMPTY_CONFIG = {
  path: "~/.config/proton-sync/proton-sync.toml",
  exists: false,
  toml: "",
  local_root: null,
  remote_root: null,
  scan_interval_secs: null,
  events_driven: null,
  include: [],
  exclude: [],
  proton_cli: null,
  proton_timeout_secs: null,
  proton_list_attempts: null,
  delete_approval_remote: null,
  delete_approval_local: null,
  // Not null: an absent `[delete_approval]` table means the daemon asks about every deletion
  // (`unwrap_or(true)` in config.rs), which is exactly what `get_deletion_policy` reports. A `null`
  // here would make the empty-config case the one shape the real command never sends.
  deletion_policy: "ask_every_time",
  // Not null, for the same reason: an absent `local_delete_mode` means the daemon trashes, which is
  // what `get_local_delete_mode` reports. A `null` here would make the empty-config case — the one
  // every existing install is in — the one shape the real command never sends.
  local_delete_mode: "trash",
};

// ---- browser-preview mock (never runs inside Tauri) ----
// `?frame=<label>` swaps the generic mock for that frame's fixture (F9), so the same dataset drives
// the fidelity harness and the design preview. Without a frame the generic mock below still runs,
// which is what keeps the browser preview useful before every frame has a fixture.
function mockInvoke(cmd, args) {
  const fixture = activeFixture();
  if (fixture) {
    // ONE COMMAND PER LINE, and a fixture key ONLY where the reply is not already inside the status.
    // A command a fixture says nothing about falls through to the generic mock below, which is what
    // keeps a partly-described frame useful rather than blank.
    switch (cmd) {
      case "get_status":
        return Promise.resolve(fixture.status);
      case "scan_conflicts":
        return Promise.resolve(fixture.conflicts ?? []);
      case "list_pending_deletions":
        // NOT A REPLY OF ITS OWN. `commands.rs` sends a plain `Status` and returns
        // `response.pending_deletions` from it, so on a real daemon these two are the same bytes by
        // construction. There is therefore no fixture key for it: reading through is the only thing
        // that cannot drift, and a top-level `deletions` would be a second source of truth for one
        // list — which is exactly the thing it would eventually disagree with.
        //
        // The first version accepted a `deletions` key and preferred it, under a comment claiming
        // the two "cannot be made to disagree". They could; the comment described the daemon and the
        // code described the fixture.
        return Promise.resolve(fixture.status?.response?.pending_deletions ?? []);
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
        // A frame that describes no config file still gets a WELL-FORMED reply, because
        // `read_config` cannot fail to send one: it stats the path, loads the doc (an absent file
        // loads as an empty doc, not an error) and fills in every field — `exists: false`, an empty
        // `toml`, `null` for each optional and `[]` for the two arrays. `{}` would leave
        // `config.exclude` undefined, so a screen doing `.map` over it would throw in browser
        // preview and nowhere else, which is the worst place for a difference to live.
        //
        // The 38 frames without an explicit config lose nothing either way: the footer's folder pair
        // reads the STATUS first (`app.js`'s `live?.local_root ?? configInfo?.local_root`), which is
        // the correct precedence anyway — a running daemon's roots are ground truth and the file is
        // the fallback.
        return Promise.resolve(fixture.config ?? EMPTY_CONFIG);
      case "read_conflict_pair":
        if (fixture.conflictPair) return Promise.resolve(fixture.conflictPair);
        break;
      case "run_dry_run":
        if (fixture.dryRun) return Promise.resolve(fixture.dryRun);
        break;
      case "free_space":
        if (fixture.freeSpace) return Promise.resolve(fixture.freeSpace);
        break;
      case "check_cli":
        if (fixture.cli) return Promise.resolve(fixture.cli);
        break;
      case "skip_rule_usage":
        if (fixture.skipRules) return Promise.resolve(fixture.skipRules);
        break;
      case "read_notify_policy":
        // A frame that says nothing about the policy is the DEFAULT, not an unanswered question:
        // `gui_prefs::load_notify_policy` cannot fail — a missing, unreadable or unknown value all
        // read back as `only_when_needed`, which is the card `11a Settings` draws chosen.
        return Promise.resolve(fixture.ui?.notifyPolicy ?? "only_when_needed");
      case "path_sync_status":
        // Keyed by the path asked for. An unlisted path answers `tracked: false` rather than falling
        // through to the generic mock: "this frame does not describe that file" is a real answer, and
        // it is the one a never-synced file gets from the real command.
        if (fixture.pathStatus)
          return Promise.resolve(fixture.pathStatus[args?.relativePath] ?? { tracked: false });
        break;
      case "search_files": {
        // Served from the SAME keyed table `path_sync_status` uses, ranked and capped the way the
        // Rust does, so a fixture describes its files once and the preview cannot show a shape the
        // real command never produces. A frame with no table answers "nothing matched", which is a
        // real answer too.
        // TRIMMED, NOT FOLDED. The reply's `query` is what the screen names the file by when nothing
        // matched, and the Rust returns the resolved query with its case intact — a lowercased one
        // would head a miss card with `spec.md` for someone who typed `Spec.md`. Folding belongs in
        // the comparisons below and nowhere else.
        const query = String(args?.query ?? "").trim();
        const folded = query.toLowerCase();
        const table = fixture.pathStatus ?? {};
        // `index_read::MatchRank`, in the same order: exact path (byte-exact first, then folded),
        // exact name, trailing components, then any fragment.
        const rankOf = (path) => {
          const foldedPath = path.toLowerCase();
          const name = foldedPath.split("/").pop();
          if (path === query) return 0;
          if (foldedPath === folded) return 1;
          if (name === folded) return 2;
          if (foldedPath.endsWith(folded) && foldedPath.at(-folded.length - 1) === "/") return 3;
          if (name.includes(folded)) return 4;
          if (foldedPath.includes(folded)) return 5;
          return null;
        };
        const ranked = !query
          ? []
          : Object.entries(table)
              .map(([path, status]) => ({ path, status, rank: rankOf(path) }))
              .filter((m) => m.rank != null)
              .sort((a, b) => a.rank - b.rank || a.path.length - b.path.length || (a.path < b.path ? -1 : 1));
        // TOTAL BEFORE THE CAP. The two are what the chooser's `Showing N of M` is made of, so a
        // mock that returned the capped length for both could never draw the line at all.
        // Clamped like `search_files` does (1..=500), so a preview cannot see a list the real
        // command would never return.
        const limit = Math.min(500, Math.max(1, args?.limit ?? 50));
        return Promise.resolve({
          matches: ranked.slice(0, limit).map(({ path, status }) => ({ path, status })),
          total: ranked.length,
          query,
        });
      }
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
    case "check_cli":
      // The silent-precondition-passes case. `null` here would be worse than useless: `check_cli`
      // runs BEFORE onboarding has a config, so S7 calls it on every `9a` frame, and every frame
      // but `9a CLI missing` would hand the screen a null to dereference.
      return Promise.resolve({ installed: true, distro: null });
    case "free_space":
      return Promise.resolve({
        available: 214_000_000_000,
        total: 500_000_000_000,
        measured_at: "/home/u",
      });
    case "skip_rule_usage":
      // THE FRAME'S OWN REPORT FIRST, exactly as `read_config` serves `fixture.config`. `7a Never
      // synced` and `7a Activity quiet` both describe a machine with a `*.tmp` rule hiding two
      // files, and without this the screen asks a mock that answers "nothing is hidden" — so the
      // band never appears and the dialog's body is empty. Both are then unmapped rather than
      // wrong, which the style gate reports as green.
      if (fixture?.skipRules) return Promise.resolve(fixture.skipRules);
      // A well-formed empty report, the way `read_config` answers with EMPTY_CONFIG: a frame that
      // describes no skip rules still gets every field, so a screen mapping over `rules` does not
      // throw in browser preview and nowhere else.
      return Promise.resolve({
        rules: (args?.patterns ?? []).map((pattern) => ({
          pattern,
          files: 0,
          bytes: 0,
          unique_files: 0,
          unique_bytes: 0,
          samples: [],
          folder_exists: null,
          error: null,
        })),
        total_files: 0,
        total_bytes: 0,
        considered_files: 0,
        unreadable_directories: 0,
        unreadable_entries: 0,
      });
    case "start_service":
      return Promise.resolve("asked systemd to start proton-syncd (preview mock)");
    case "open_paths":
    case "open_folder":
    case "open_remote":
    case "open_system_log":
      // A browser tab has no `xdg-open` and no journal. Resolving is the honest preview: the click
      // is the thing being looked at, and rejecting would draw a failure nobody chose to see —
      // `?frame=` has no way to ask for one, unlike `8a Save refused`'s fixture-driven error.
      return Promise.resolve(null);
    case "read_notify_policy":
      return Promise.resolve("only_when_needed");
    case "write_notify_policy":
    case "close_notification":
      return Promise.resolve(null);
    case "send_notification":
      // A browser has no notification server, and drawing one here would be the preview inventing a
      // surface. `?frame=11a Outage` is where a banner is looked at.
      return Promise.resolve(null);
    case "write_config":
      // Accepts. The REFUSAL is what `8a Save refused` is for, and it is reached by the fixture's
      // own `saveError` rather than by a mock that decides to fail — a preview that rejected every
      // fourth save would be a design surface nobody could look at on purpose.
      return Promise.resolve(null);
    case "resync":
      return Promise.resolve({
        state: "running",
        response: { status: "syncing", paused: false, syncing: true, message: "full sweep queued" },
        error: null,
      });
    case "choose_folder":
      // A dismissed picker, which is what a browser has to be: there is no native dialog here, and
      // answering with a plausible path would stage a folder change nobody chose.
      return Promise.resolve(null);
    case "approve":
    case "deny":
    case "keep":
      // Simulate the daemon round trip so the Deletions screen's busy → settled flow is visible in
      // browser preview. Shaped like a real StatusPayload, INCLUDING the acknowledgement message:
      // the screen requires the daemon's own `approved N …` / `denied N …` / `kept N …` wording
      // before it treats anything as decided, because `apply_approval_command` answers
      // `Ok("no pending deletion matches …")` for a selector it cannot find. A mock without the
      // message is a mock the screen correctly refuses, which would look like a broken preview.
      return new Promise((resolve) => {
        // `kept` has its own tail, as the daemon's does: keeping restores the other side on the
        // next pass rather than asking for a `syncnow` (the command schedules one itself).
        const message =
          cmd === "keep"
            ? "kept 1 pending deletion(s); the other side is put back on the next sync"
            : `${cmd === "approve" ? "approved" : "denied"} 1 pending deletion(s); run \`proton-sync syncnow\` to apply now`;
        setTimeout(
          () =>
            resolve({
              state: "running",
              response: { paused: false, message },
              error: null,
            }),
          800,
        );
      });
    case "restart_service":
      // Simulate the real stop→start latency so the Settings screen's "Restarting…" state is
      // visible in browser preview. THE SHAPE IS THE COMMAND'S, not a string: the save path branches
      // on the typed `ending`, and a mock resolving with prose — or with #328's `{ restarted }`,
      // which nothing reads any more — would exercise a path the app does not have (#320/#335).
      return new Promise((resolve) => {
        setTimeout(
          () => resolve({ ending: "restarted", detail: "the service restarted (preview mock)" }),
          1200,
        );
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
      return Promise.resolve([
        { original: "notes/todo.txt", sidecar: "notes/todo.proton-cloud.txt", kind: "content" },
      ]);
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
