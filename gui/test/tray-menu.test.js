// The tray menu at two folders or more (#102 phase 5d) — `folderMenuRows`, the panel half of a menu
// the native right-click menu (`src-tauri/src/tray_menu.rs`) and the fallback text menu also draw.
//
// THREE SURFACES, ONE MENU. The panel's rows are JS, the native rows are Rust, and no gate in either
// language can see the other. `tray-menu-corpus.json` is what holds them together: this file asserts
// the panel's rows against it and `tray_menu.rs`'s `the_native_rows_are_the_corpus_the_panel_is_held_to`
// asserts the native ones against the same file, id, label and sub-label, rules included. A row added
// on one side only fails the other side's test, and the corpus is the thing a reviewer reads.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import { folderMenuRows, menuSignature, TRAY_MENU, TRAY_FOLDER_CAP } from "../src/js/ui/compact.js";
import { TRAY } from "../src/js/ui/copy.js";

const CORPUS = JSON.parse(
  readFileSync(fileURLToPath(new URL("./tray-menu-corpus.json", import.meta.url)), "utf8"),
);

/**
 * The panel's menu key for a daemon state. The test's own table and deliberately not an import: the
 * corpus is written in the daemon's vocabulary (`DaemonState`, which Rust serializes) and the panel's
 * is its own (`TRAY_MENU`'s keys), and this is the mapping between them that `screens/tray.js` makes
 * through the hero. Written out so a state the panel maps differently is a failure here.
 */
const MENU_KEY = {
  idle: "settled",
  running: "syncing",
  paused: "paused",
  unreachable: "notRunning",
  authExpired: "deferToWindow",
  failed: "outage",
  firstRun: "deferToWindow",
};

const asCorpusRow = (row) => (row.separator ? "-" : [row.id, row.label, row.sub ?? null]);

test("the corpus is not empty and names every daemon state", () => {
  assert.ok(CORPUS.length >= 15, `only ${CORPUS.length} cases`);
  const seen = new Set(CORPUS.map((entry) => entry.state));
  for (const state of Object.keys(MENU_KEY)) assert.ok(seen.has(state), `no case for ${state}`);
});

for (const entry of CORPUS) {
  test(`the panel draws the corpus rows: ${entry.case}`, () => {
    const rows = folderMenuRows(MENU_KEY[entry.state], entry.pairs);
    assert.deepEqual(rows.map(asCorpusRow), entry.rows);
  });
}

test("the needs-you menu is the settled one, at any number of folders", () => {
  // `needsYou` has no daemon state of its own — the panel's decision button is not a menu row — so the
  // two keys must draw the same rows, or a pending deletion would change the pause rows.
  for (const entry of CORPUS.filter((e) => e.state === "idle")) {
    assert.deepEqual(
      folderMenuRows("needsYou", entry.pairs).map(asCorpusRow),
      folderMenuRows("settled", entry.pairs).map(asCorpusRow),
      entry.case,
    );
  }
});

test("at one folder, or none, the rows are the fixed table's own objects", () => {
  // The N=1 promise (D2) is that nothing about the rows changes. Identity is the strongest form of
  // "the same": the very rows `TRAY_MENU` has, so no copy can have drifted.
  for (const key of Object.keys(TRAY_MENU)) {
    assert.equal(folderMenuRows(key, []), TRAY_MENU[key]);
    assert.equal(
      folderMenuRows(key, [{ name: "docs", paused: false, syncing: false, rank: 0 }]),
      TRAY_MENU[key],
    );
  }
});

test("no row at two folders says Pause syncing, and none pauses everything", () => {
  const folders = [
    { name: "documents", paused: false, syncing: false, rank: 0 },
    { name: "photos", paused: true, syncing: false, rank: 1 },
  ];
  for (const key of Object.keys(TRAY_MENU)) {
    for (const row of folderMenuRows(key, folders)) {
      if (row.separator) continue;
      assert.notEqual(row.label, TRAY.pause, `${key}: ${row.id}`);
      assert.notEqual(row.label, TRAY.resume, `${key}: ${row.id}`);
      assert.ok(!/all/i.test(row.id), `${key}: ${row.id} reads like a pause-everything row`);
      assert.notEqual(row.id, "pause", key);
      assert.notEqual(row.id, "resume", key);
    }
  }
});

test("the panel draws at most five folders worst first, then one row that says how many are left", () => {
  const folders = ["a", "b", "c", "d", "e", "f", "g"].map((name) => ({
    name,
    paused: false,
    syncing: false,
    // `g` is the worst; the rest tie, so the daemon's order breaks it.
    rank: name === "g" ? 4 : 0,
  }));
  const rows = folderMenuRows("settled", folders, { cap: TRAY_FOLDER_CAP });
  const group = rows.filter((row) => row.id?.includes("@") || row.id === "more");
  assert.deepEqual(
    group.map((row) => row.id),
    ["pause@g", "pause@a", "pause@b", "pause@c", "pause@d", "more"],
  );
  assert.equal(group.at(-1).label, TRAY.moreFolders(2));
  assert.equal(group.at(-1).label, "2 more folders");
  // The native menus have no cap: every folder, and no `more` row.
  const all = folderMenuRows("settled", folders);
  assert.equal(all.filter((row) => row.id?.includes("@")).length, 7);
  assert.ok(!all.some((row) => row.id === "more"));
});

test("exactly five folders draw no more-row, and six draw the singular", () => {
  const some = (n) =>
    Array.from({ length: n }, (_, at) => ({ name: `f${at}`, paused: false, syncing: false, rank: 0 }));
  assert.ok(!folderMenuRows("settled", some(5), { cap: TRAY_FOLDER_CAP }).some((row) => row.id === "more"));
  const six = folderMenuRows("settled", some(6), { cap: TRAY_FOLDER_CAP });
  assert.equal(six.find((row) => row.id === "more").label, "1 more folder");
});

test("a folder's row id carries its folder, whatever the folder is called", () => {
  // `@` is outside the folder-name charset `[A-Za-z0-9._-]`, so splitting on it is unambiguous and a
  // folder called `pause` or `quit` is `pause@pause` and `pause@quit`: never the verb itself.
  const rows = folderMenuRows("settled", [
    { name: "pause", paused: false, syncing: false, rank: 0 },
    { name: "quit", paused: true, syncing: false, rank: 1 },
  ]);
  const ids = rows.filter((row) => row.id?.includes("@")).map((row) => row.id);
  assert.deepEqual(ids, ["resume@quit", "pause@pause"]);
});

test("the menu's signature changes when ONE folder's pause flag does", () => {
  // `updateCompactPanel` patches the text of a panel whose menu signature is unchanged, and rebuilds
  // otherwise. The rows' handlers are bound at build time, so a panel patched across a pause would
  // keep a `Pause photos` row that `Resume photos` should have replaced.
  const before = [
    { name: "documents", paused: false, syncing: false, rank: 0 },
    { name: "photos", paused: false, syncing: false, rank: 0 },
  ];
  const after = [before[0], { ...before[1], paused: true, rank: 1 }];
  assert.notEqual(
    menuSignature(folderMenuRows("settled", before)),
    menuSignature(folderMenuRows("paused", after)),
  );
  // Same state key, only the flag moved: still different.
  assert.notEqual(
    menuSignature(folderMenuRows("settled", before)),
    menuSignature(folderMenuRows("settled", after)),
  );
});

test("the labels are the copy deck's templates", () => {
  const rows = folderMenuRows("settled", [
    { name: "documents", paused: false, syncing: false, rank: 0 },
    { name: "photos", paused: true, syncing: false, rank: 1 },
  ]);
  const byId = Object.fromEntries(rows.filter((row) => !row.separator).map((row) => [row.id, row.label]));
  assert.equal(byId["pause@documents"], TRAY.pausePair("documents"));
  assert.equal(byId["resume@photos"], TRAY.resumePair("photos"));
  assert.equal(TRAY.pausePair("documents"), "Pause documents");
  assert.equal(TRAY.resumePair("documents"), "Resume documents");
});
