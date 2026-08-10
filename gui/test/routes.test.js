// The route table's invariants. Two of these are promises the design makes in writing, and a
// promise nothing checks is a promise that drifts:
//
//   · "Footer's four doors never move or reorder" — 14-behaviour-and-state.md's testing checklist.
//   · Every screen has EITHER the doors OR a footer action bar, never both — measured over the 22
//     in-scope 1040 frames, DEVIATIONS.md §40.
//
// Plus the tray aliases, which exist because tray.rs is Rust and does not move when this table does.

import { test } from "node:test";
import assert from "node:assert/strict";
import { ROUTES, FOOTER_ORDER, isOverlay, isDialog, ROUTE_ALIASES, resolveRoute } from "../src/js/routes.js";

test("the four doors, in this order, and no others", () => {
  // Hard-coded rather than derived: deriving it from ROUTES would let a reordering of the object
  // literal reorder the footer, which is the exact thing the checklist forbids.
  assert.deepEqual(FOOTER_ORDER, ["activity", "plan", "settings", "details"]);
});

test("every footer entry is a real route with a label to show", () => {
  for (const id of FOOTER_ORDER) {
    assert.ok(ROUTES[id], `${id} is in the footer but not in ROUTES`);
    assert.equal(typeof ROUTES[id].label, "string", `${id} has no label`);
  }
});

test("Details is a footer label but an overlay, not a destination", () => {
  // 5a/6a draw it as a panel over the screen you were on. Making it a door would lose your place.
  assert.ok(isOverlay("details"));
  assert.ok(FOOTER_ORDER.includes("details"));
});

test("Conflicts and Deletions are overlays, not doors", () => {
  // They are reached from the attention band, the status chip or a notification. A door for each is
  // what the v1 sidebar did, and the badge counts that came with it.
  for (const id of ["conflicts", "deletions"]) {
    assert.ok(isOverlay(id), `${id} should be an overlay`);
    assert.ok(!FOOTER_ORDER.includes(id), `${id} must not be in the footer`);
  }
});

test("every route declares a known kind", () => {
  for (const [id, spec] of Object.entries(ROUTES)) {
    assert.ok(["root", "door", "overlay"].includes(spec.kind), `${id} has kind "${spec.kind}"`);
  }
});

test("a route's footer is doors or an action bar — never both, never something else", () => {
  for (const [id, spec] of Object.entries(ROUTES)) {
    if (spec.footer === undefined) continue; // overlays inherit whatever is underneath
    assert.ok(["doors", "actionBar"].includes(spec.footer), `${id} has footer "${spec.footer}"`);
  }
});

test("the screens that commit something have an action bar instead of doors", () => {
  // The measured 13-to-6 split. Settings, Plan and onboarding therefore have no navigation at all,
  // which is why the app mark has to be a home affordance.
  for (const id of ["plan", "settings", "onboarding"]) {
    assert.equal(ROUTES[id].footer, "actionBar", `${id} should carry the action bar`);
  }
  assert.equal(ROUTES.main.footer, "doors");
});

test("only onboarding is a takeover, and it is not dismissible", () => {
  const takeovers = Object.entries(ROUTES).filter(([, s]) => s.takeover);
  assert.deepEqual(
    takeovers.map(([id]) => id),
    ["onboarding"],
  );
});

test("the tray's legacy ids resolve to routes that exist", () => {
  // tray.rs emits exactly three: settings, conflicts, and history. The first two still resolve on
  // their own; `history` does not, because design-v2 has no History screen — its two jobs moved
  // into Activity. Without the alias the tray's "View journal" silently does nothing.
  for (const emitted of ["settings", "conflicts", "history"]) {
    assert.ok(ROUTES[resolveRoute(emitted)], `tray emits "${emitted}" and nothing resolves it`);
  }
  assert.equal(resolveRoute("history"), "activity");
});

test("every alias points at a route that exists", () => {
  for (const [from, to] of Object.entries(ROUTE_ALIASES)) {
    assert.ok(ROUTES[to], `alias ${from} -> ${to}, which is not a route`);
    assert.ok(!ROUTES[from], `${from} is an alias AND a route — the alias would never be reached`);
  }
});

test("an unknown id passes through unchanged, for the caller to reject", () => {
  assert.equal(resolveRoute("somethingElse"), "somethingElse");
});

// ---- the dialog/screen split (F5) ----
//
// `presentation` is what decides whether an overlay floats behind a scrim or replaces the screen's
// body, and it was measured rather than chosen: a dialog is a standalone surface with no app header
// and no footer doors, while the other overlays are full 1042x766 windows that keep both. Asserted
// here because the two are one word apart in the table and the wrong one is not a crash — it is a
// scrim over a screen that should have been replaced, or a full window with no way back.

// `filePending` joined the set with S5. It is the fourth and, unlike the other three, it draws no
// title row and no ✕ — which is a fact about its CONTENTS, not about how it is presented, so it is
// still a dialog by this test's measure: a floating surface with no header and no footer doors.
test("only the four drawn dialogs are dialogs", () => {
  const dialogs = Object.keys(ROUTES).filter((id) => isDialog(id));
  assert.deepEqual(dialogs.sort(), ["details", "filePending", "neverSynced", "saveRefused"]);
});

test("every overlay declares a presentation, and no door does", () => {
  for (const [id, spec] of Object.entries(ROUTES)) {
    if (spec.kind !== "overlay") {
      assert.equal(spec.presentation, undefined, `${id} is not an overlay and must not declare one`);
      continue;
    }
    // Onboarding is the exception: a takeover is neither, and routes.js says why.
    if (spec.takeover) continue;
    assert.ok(
      spec.presentation === "dialog" || spec.presentation === "screen",
      `${id} must declare presentation: "dialog" or "screen"`,
    );
  }
});

test("the onboarding takeover is not a dialog", () => {
  // A scrim would darken a screen nobody can reach, and Esc must not close it.
  assert.equal(isDialog("onboarding"), false);
  assert.equal(ROUTES.onboarding.takeover, true);
});

test("every dialog carries the drawn size, not the declared one", () => {
  // §48a: four of the ten drawn dialogs opt into border-box and six do not, so 520 is drawn 522
  // while 600 is drawn 600. There is no offset to apply — only a number read off the frame.
  const drawn = { details: [522, 462], neverSynced: [602, 602], saveRefused: [600, null] };
  for (const [id, size] of Object.entries(drawn)) {
    assert.deepEqual(ROUTES[id].size, size, `${id} must carry its drawn box`);
  }
});

test("the ✕ is per dialog, and the refusal has none", () => {
  // `8a Save refused` and `9a CLI missing` draw no close button: they ask you to choose between two
  // repairs, and a dismiss in the corner is a third answer the design does not offer. Esc still
  // closes them — the ✕ is the pointer affordance, not the only way out.
  assert.equal(ROUTES.details.closable, true);
  assert.equal(ROUTES.neverSynced.closable, true);
  assert.equal(ROUTES.saveRefused.closable, false);
  for (const id of Object.keys(ROUTES).filter(isDialog)) {
    assert.equal(typeof ROUTES[id].closable, "boolean", `${id} must say whether it has a ✕`);
  }
});
