// Per-frame fixtures (F9) — one deterministic dataset per in-scope frame label, selected by
// `?frame=<label>`.
//
// The same data drives the fidelity harness and the browser design preview, so a frame that passes
// CI is a frame a human can open and look at. That is the whole point: a gate whose inputs nobody
// can see is a gate nobody trusts.
//
// STATE OF PLAY. Every route is a placeholder until S1–S11 land, so a fixture can only feed what
// exists — today that is the shell F4 built. The three main-screen frames below are complete: their
// data drives the header and footer, and their `fids` map those nodes onto the drawn frame so
// assert.mjs has something real to compare. Each S-task adds its screens' rows.
//
// `fids` is the hand-written half of the mapping the F8 issue accepts as unavoidable. extract.mjs
// derives a key per drawn node (`header/span[2]`, `div[2]/div[0]/span[1]`); nothing can derive which
// APP node corresponds, because the two trees differ by design. So each frame says it here, once,
// and ui/chrome.js stamps `data-fid` from it.

/** The frame the URL asks for, or null for the live app. */
export function activeFrame() {
  if (typeof location === "undefined") return null;
  return new URLSearchParams(location.search).get("frame");
}

/**
 * Where the shell's own slots live in each frame's node tree. The footer's key differs per frame
 * because the number of blocks above it does — `2a Settled` has one hero, `2a Needs you` has a hero
 * plus a transfer grid plus an attention band — which is exactly why this cannot be derived.
 */
const SHELL_FIDS = {
  "2a Settled": {
    header: "header",
    mark: "header/img",
    name: "header/span[0]",
    spacer: "header/span[1]",
    chip: "header/span[2]",
    chipDot: "header/span[2]/span",
    menu: "header/button",
    footerNav: "div[2]",
    footerBar: "div[2]/div[0]",
    door: (i) => `div[2]/div[0]/span[${i}]`,
    footerLine: "div[2]/div[1]",
  },
  "2a Syncing": {
    header: "header",
    mark: "header/img",
    name: "header/span[0]",
    spacer: "header/span[1]",
    chip: "header/span[2]",
    chipDot: "header/span[2]/span",
    menu: "header/button",
    footerNav: "div[2]",
    footerBar: "div[2]/div[0]",
    door: (i) => `div[2]/div[0]/span[${i}]`,
    footerLine: "div[2]/div[1]",
  },
  "2a Needs you": {
    header: "header",
    mark: "header/img",
    name: "header/span[0]",
    spacer: "header/span[1]",
    chip: "header/span[2]",
    chipDot: "header/span[2]/span",
    menu: "header/button",
    footerNav: "div[3]",
    footerBar: "div[3]/div",
    door: (i) => `div[3]/div/span[${i}]`,
  },
};

/**
 * The datasets. Shaped exactly like the daemon's `StatusPayload` so the app cannot tell a fixture
 * from a live reply — anything that needs a special case here would be a special case in the app.
 */
const now = () => Math.floor(Date.now() / 1000);

export const FIXTURES = {
  "2a Settled": {
    fids: SHELL_FIDS["2a Settled"],
    status: {
      state: "idle",
      response: {
        status: "idle",
        paused: false,
        syncing: false,
        pending_changes: 0,
        last_sync_epoch_secs: now() - 120,
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    conflicts: [],
  },
  "2a Syncing": {
    fids: SHELL_FIDS["2a Syncing"],
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: now() - 14,
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    conflicts: [],
  },
  "2a Needs you": {
    fids: SHELL_FIDS["2a Needs you"],
    status: {
      state: "running",
      response: {
        status: "syncing",
        paused: false,
        syncing: true,
        pending_changes: 3,
        last_sync_epoch_secs: now() - 14,
        pending_deletions: [],
        config: { local_root: "~/ProtonDrive", remote_root: "/Drive/RemoteFolder" },
      },
    },
    // Three decisions waiting: the chip reads `3 waiting` with the ring dot, which is what the
    // frame draws even though a transfer is also in flight (DEVIATIONS.md §44).
    conflicts: [{ path: "notes/todo.txt" }, { path: "docs/spec.md" }, { path: "a/b.txt" }],
  },
};

/** The fixture for the selected frame, or null. */
export function activeFixture() {
  const label = activeFrame();
  return label ? (FIXTURES[label] ?? null) : null;
}

/**
 * Stamp `data-fid` on a node, if a frame is selected and it has a key for this slot. A no-op in the
 * live app, so the attribute never ships to a user — it exists only for the harness and the preview.
 */
export function fid(node, slot, ...args) {
  const fixture = activeFixture();
  const key = fixture?.fids?.[slot];
  if (node && key != null) {
    node.setAttribute("data-fid", `${activeFrame()}:${typeof key === "function" ? key(...args) : key}`);
  }
  return node;
}
