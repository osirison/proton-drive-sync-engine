// The tray menu's five row sets (F6), tested on their own.
//
// This frontend is tested selectively, the way onboarding-latch.test.js explains — most of the
// compact panel is better checked by the fidelity gate, which now compares all eight of its frames
// node by node. What it cannot check is here:
//
//   · `IMPLEMENTATION-PLAN.md` §8's acceptance checklist puts "`Close window` / `Quit` sub-labels
//     present in the tray" against **unit test**, not against the harness, and it is right to: the
//     sub-labels are what stops someone quitting the daemon when they meant to close a window, and
//     the gate only sees the states a frame was drawn for.
//   · THE NEEDS-YOU MENU IS DRAWN NOWHERE. `10-tray.md` gives it a row in the table and the
//     prototype has no `10a Needs you` panel, so of the five sets the gate covers four. The one it
//     misses is the state where a person is most likely to be reaching for the tray.
//   · Two absences carry meaning and would read as omissions to anyone tidying the table: `Sync now`
//     is missing while syncing, and `Close window` is missing from both stopped states.

import { test } from "node:test";
import assert from "node:assert/strict";
import { TRAY_MENU, trayMenu } from "../src/js/ui/compact.js";

const STATES = ["settled", "syncing", "needsYou", "paused", "unreachable"];
const rowsOf = (state) => TRAY_MENU[state].filter((row) => !row.separator);
const labels = (state) => rowsOf(state).map((row) => row.label);

test("every state the panel has is a state the tray menu has", () => {
  assert.deepEqual(Object.keys(TRAY_MENU).sort(), [...STATES].sort());
});

test("Close window and Quit keep their sub-labels wherever they appear", () => {
  // The one item on the design's acceptance checklist that names a unit test.
  const subs = { "Close window": "keeps syncing", Quit: "stops syncing" };
  let seen = 0;
  for (const state of STATES) {
    for (const row of rowsOf(state)) {
      if (!(row.label in subs)) continue;
      seen++;
      assert.equal(row.sub, subs[row.label], `${state} · ${row.label}`);
    }
  }
  // Three states carry both rows, two carry only Quit: 3×2 + 2×1.
  assert.equal(seen, 8);
});

test("only those two rows carry a sub-label — a menu is not a place for explanations", () => {
  for (const state of STATES) {
    for (const row of rowsOf(state)) {
      if (row.sub) assert.ok(["Close window", "Quit"].includes(row.label), `${state} · ${row.label}`);
    }
  }
});

test("Sync now is absent while syncing, because it would do nothing", () => {
  assert.ok(!labels("syncing").includes("Sync now"));
  assert.ok(labels("settled").includes("Sync now"));
  assert.ok(labels("needsYou").includes("Sync now"));
});

test("the two stopped states drop Close window rather than promise it keeps syncing", () => {
  for (const state of ["paused", "unreachable"]) {
    assert.ok(!labels(state).includes("Close window"), state);
    assert.ok(labels(state).includes("Quit"), state);
  }
});

test("each stopped state leads with the row that fixes it", () => {
  assert.equal(labels("paused")[0], "Resume syncing");
  assert.equal(labels("unreachable")[0], "Try again now");
});

test("the panel's own Review them is not repeated in the needs-you menu", () => {
  assert.ok(!labels("needsYou").includes("Review them"));
});

test("every menu has exactly one separator, and it never sits at either end", () => {
  for (const state of STATES) {
    const rows = TRAY_MENU[state];
    const at = rows.map((row, i) => (row.separator ? i : -1)).filter((i) => i >= 0);
    assert.equal(at.length, 1, state);
    assert.ok(at[0] > 0 && at[0] < rows.length - 1, state);
  }
});

test("trayMenu binds one handler per row and leaves the separator alone", () => {
  const picked = [];
  const rows = trayMenu("settled", (id) => picked.push(id));
  for (const row of rows) {
    if (row.separator) {
      assert.equal(row.onClick, undefined, "a rule is not a menu item");
      continue;
    }
    row.onClick();
  }
  assert.deepEqual(picked, ["open", "syncNow", "pause", "closeWindow", "quit"]);
});

test("trayMenu does not mutate the table it reads", () => {
  trayMenu("settled", () => {});
  assert.ok(TRAY_MENU.settled.every((row) => row.onClick === undefined));
});

test("an unknown state throws rather than returning an empty menu", () => {
  // A tray that renders no rows looks like a tray that is still loading, and stays that way.
  assert.throws(() => trayMenu("offline"), /no tray menu for state "offline"/);
});
