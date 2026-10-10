// The datasets of adding, listing and removing folders (#102 phase 5c-2) — four dark frames.
//
//   8a Folders list       the Folders tab at two folders: the list above the settings it chooses between
//   8a Save two folders   the action bar when a save costs two folders their syncing
//   8a Add folder         the dialog, checked: both sides priced, an index it would resume, no preview
//   8a Remove folder      the confirmation for the FIRST folder (the one that names a new default)
//
// ALL FOUR LIST TWO FOLDERS, because none of them exists below two except the add dialog — and the add
// dialog's frame is drawn over a Settings window that has two, which is where it is most often opened
// from. `check-n1-identity.mjs` therefore does not rewrite them as one folder (a frame that lists two has
// no one-folder rendering to be identical to), and what holds the one-folder app is every other frame.
//
// TWO OF THEM ARE CROPS, and a crop's fixture is still the WHOLE app: the harness renders the Settings
// window and maps only the nodes the 600px drawing has, so the picture a reviewer gets from
// `npm run screenshots -- --frame "8a Folders list"` is the window the list is part of.
//
// THE FLOW STATE IS THE FIXTURE'S (`addFolder`, `removeFolder`): the add dialog is a sequence — typing, a
// check, the add — and `?frame=` cannot press anything, so the frame names the state it draws. The state is
// exactly the one `app.js` would hold: the engine's reply to `check_add_pair`, the two probes' replies, and
// the key of the inputs they answered (`addKeyOf`, written out here because this module may not import
// `folders.js`, which imports the selector, which imports the registry that imports this).

import { settledStatus, stateOf, summary, configListing } from "./pairs.js";
import { foldersFids } from "./fids.js";

const PAIRS = [summary("documents"), summary("photos", { syncing: true, pending_changes: 3 })];
const STATES = [stateOf("documents", "idle", 0), stateOf("photos", "running", 3)];

const STATUS = settledStatus(PAIRS, STATES);
const CONFIG = configListing(["documents", "photos"]);

/** What `check_add_pair` said about the folder this frame adds: the name is the folder's own, and its index survived. */
const CHECKED = {
  suggested_name: "photos",
  name_error: null,
  refusal: null,
  surviving_index: {
    path: "~/Photos/.sync/sync_index.db",
    set_aside_pending: false,
    message:
      "This folder already holds sync history from an earlier setup (~/Photos/.sync/sync_index.db); adding it resumes from that history, so anything changed since may show up as deletions to approve. To start fresh instead, run proton-sync reset-index --yes --pair photos after adding. This app cannot reset an index.",
  },
  warnings: [],
};

/** `addKeyOf` of `photos`, `~/Photos`, `/Drive/Photos`: the inputs the check below answered. */
const KEY = JSON.stringify(["photos", "~/Photos", "/Drive/Photos"]);

export const FOLDER_FIXTURES = {
  "8a Folders list": {
    status: STATUS,
    config: CONFIG,
    conflicts: [],
    route: "settings",
    ui: { tab: "folders" },
    fids: foldersFids("list"),
  },

  // `ui.dirty` is what a staged change is to the bar (the same flag `8a Skip rules` uses): a frame cannot
  // type, so it names the state. The cost line is absent, so the note is the restart sentence alone.
  "8a Save two folders": {
    status: STATUS,
    config: CONFIG,
    conflicts: [],
    route: "settings",
    ui: { tab: "folders", dirty: true },
    fids: foldersFids("save"),
  },

  "8a Add folder": {
    status: STATUS,
    config: CONFIG,
    conflicts: [],
    route: "settings",
    ui: { tab: "folders", dialog: "addFolder" },
    addFolder: {
      phase: "form",
      name: "photos",
      nameTouched: false,
      local: "~/Photos",
      remote: "/Drive/Photos",
      rules: [],
      draft: "",
      pre: CHECKED,
      preKey: KEY,
      probes: {
        local: { files: 1204, bytes: 3_400_000_000, truncated: false, unreadable_directories: 0 },
        // The Proton side reports files and no size: `bytes` is `null` and the line says only the count.
        remote: { files: 1190, bytes: null, truncated: false, unreadable_directories: 0 },
      },
      checkedKey: KEY,
      seq: 0,
      error: null,
      ending: null,
      reason: "",
      waits: 0,
      seenIssue: 0,
    },
    fids: foldersFids("add"),
  },

  "8a Remove folder": {
    status: STATUS,
    config: CONFIG,
    conflicts: [],
    route: "settings",
    ui: { tab: "folders", dialog: "removeFolder" },
    removeFolder: { pair: "documents", phase: "confirm", reply: null, error: null },
    fids: foldersFids("remove"),
  },
};
