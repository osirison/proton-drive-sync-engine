// The banner builders (S9) — and the one rule this screen exists to state.
//
// `11-notifications.md`: "Nothing destructive is ever a notification button. Delete, discard a
// version, approve all — none of them appear in a banner." That is the DEFINITION OF DONE for #188,
// and it is the kind of property that holds until someone adds a fifth event in a hurry. `safeActions`
// enforces it at construction; this proves the enforcement can actually fire, which is the half a
// guard cannot prove about itself.
//
// The builders are pure and the renderer is not, so everything here drives `bannerFor`/`payloadFor`.
// What the banner LOOKS like is the fidelity gate's question — five frames, mapped.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  EVENT_KINDS,
  SAFE_ACTIONS,
  bannerFor,
  isDestructive,
  payloadFor,
} from "../src/js/ui/notification.js";
import { DELETIONS, NOTIFY } from "../src/js/ui/copy.js";
import { readFileSync } from "node:fs";

/** One event of each kind, at the data the frames draw. */
const EVENTS = {
  deletion: { kind: "deletion", paths: ["photos/2019"], entity: "folder" },
  conflict: { kind: "conflict", paths: ["notes/todo.txt"] },
  firstSync: { kind: "firstSync" },
  outage: { kind: "outage", changes: 61 },
};

test("no banner offers a destructive action", () => {
  for (const kind of EVENT_KINDS) {
    for (const action of bannerFor(EVENTS[kind]).actions) {
      assert.ok(SAFE_ACTIONS.has(action.id), `${kind} offers "${action.id}"`);
      assert.ok(!isDestructive(action.label), `${kind} offers "${action.label}"`);
    }
  }
  // The grouped form is a different branch of the same builder and has its own pair of actions.
  for (const action of bannerFor({ kind: "conflict", paths: ["a", "b", "c"] }).actions) {
    assert.ok(SAFE_ACTIONS.has(action.id));
    assert.ok(!isDestructive(action.label));
  }
});

test("the guard catches the words the deletions screen uses for the same acts", () => {
  // THE HALF A GUARD CANNOT PROVE ABOUT ITSELF. Everything above passes on a regex that matches
  // nothing, so the vocabulary is checked against the deck's own destructive strings — the four
  // labels S3 puts on the buttons that do these things.
  for (const label of [
    DELETIONS.delete,
    DELETIONS.armedConfirm,
    // The other two acts the hard rule names by name. Neither is a deck string — no screen offers
    // "approve all" as a button — so they are written here as the sentence writes them.
    "Approve all",
    "Discard this version",
  ]) {
    assert.ok(isDestructive(label), `"${label}" is destructive and the guard did not say so`);
  }
  // And it does not flag the safe ones, or the guard would refuse every banner and be untestable
  // from the other side.
  for (const label of [NOTIFY.deletionKeep, NOTIFY.deletionReview, NOTIFY.conflictCompare, NOTIFY.later]) {
    assert.ok(!isDestructive(label), `"${label}" is safe and the guard flagged it`);
  }
});

test("the safe action leads, and the one good-news banner offers none", () => {
  // `11-notifications.md`: "note the safe action is primary and there is no Delete".
  const deletion = bannerFor(EVENTS.deletion);
  assert.equal(deletion.actions[0].label, NOTIFY.deletionKeep);
  assert.equal(deletion.actions[0].role, "primary");
  assert.equal(deletion.actions.length, 2);
  assert.deepEqual(bannerFor(EVENTS.firstSync).actions, []);
});

test("several conflicts are one banner, counted in the mark", () => {
  const one = bannerFor({ kind: "conflict", paths: ["notes/todo.txt"] });
  assert.equal(one.title, NOTIFY.conflictTitle("notes/todo.txt"));
  assert.equal(one.icon.numeral, undefined, "a single conflict draws no numeral");

  const five = bannerFor({ kind: "conflict", paths: ["a", "b", "c", "d", "e"] });
  assert.equal(five.icon.numeral, 5);
  assert.equal(five.title, NOTIFY.groupedTitle(5));
  // VERSIONS, not files: two copies per conflict, and the larger number is the reassurance.
  assert.match(five.body[0].text, /All ten versions are safe/);
});

test("the deletion body carries its path as its own segment", () => {
  // The frame breaks its text nodes around a mono `<span>`, so the sentence is two parts and not a
  // template — `renderBanner` needs to know which half is the path.
  const spec = bannerFor(EVENTS.deletion);
  assert.deepEqual(spec.body, [{ mono: "photos/2019" }, { text: NOTIFY.deletionBodyAfter }]);
  // …and flattens back to one sentence for a notification server, which takes no markup.
  assert.equal(payloadFor(spec).body, `photos/2019${NOTIFY.deletionBodyAfter}`);
});

test("a folder is not called a file", () => {
  // G8 (#208) leaves the count as the queue's length; the noun still has to be true of what is in
  // it, or the banner asks you to keep something it has misnamed.
  assert.match(bannerFor(EVENTS.deletion).title, /^1 folder would be deleted/);
  assert.match(
    bannerFor({ kind: "deletion", paths: ["a", "b"], entity: "folder" }).title,
    /^2 folders would be deleted/,
  );
  assert.match(bannerFor({ kind: "deletion", paths: ["a.txt"] }).title, /^1 file would be deleted/);
});

test("the outage banner does not blame the session when the session is not the cause", () => {
  // `11-notifications.md` gives the trigger three causes and draws one sentence. Saying "Proton
  // Drive is asking you to sign in again" about a full disk would be false in the banner whose whole
  // job is to be believed.
  assert.match(bannerFor({ kind: "outage", changes: 61, cause: "auth" }).body[0].text, /sign in again/);
  const other = bannerFor({ kind: "outage", changes: 61, cause: "unreachable" }).body[0].text;
  assert.doesNotMatch(other, /sign in again/);
  assert.match(other, /Nothing is lost/);
});

test("a payload names a tray glyph, so the banner and the indicator agree", () => {
  for (const kind of EVENT_KINDS) {
    const payload = payloadFor(bannerFor(EVENTS[kind]));
    assert.match(payload.icon, /^proton-sync-.*-symbolic$/, `${kind} has no glyph`);
    assert.equal(payload.summary, bannerFor(EVENTS[kind]).title);
  }
});

test("an unknown event is a throw, not an empty banner", () => {
  assert.throws(() => bannerFor({ kind: "nothing-like-this" }), /unknown event/);
  assert.throws(() => bannerFor(null), /unknown event/);
});

test("a payload carries every field the Rust struct requires", () => {
  // TWO LANGUAGES, ONE STRUCT, AND NOTHING BETWEEN THEM. `NotifyPayload` is `serde::Deserialize`
  // with no `#[serde(default)]` anywhere, so a missing field is not a wrong banner — it is serde
  // refusing the payload and `send_notification` returning an error for every event, for ever.
  // `app` was missing and every gate in this repo was green.
  const rust = readFileSync(new URL("../src-tauri/src/notify.rs", import.meta.url), "utf8");
  const struct = rust.slice(rust.indexOf("pub struct NotifyPayload"));
  const fields = [...struct.slice(0, struct.indexOf("}")).matchAll(/pub (\w+):/g)].map((m) => m[1]);
  assert.ok(fields.length >= 5, `parsed ${fields.length} fields — did the struct move?`);
  const payload = payloadFor(bannerFor(EVENTS.deletion));
  for (const field of fields) {
    assert.ok(payload[field] != null, `payloadFor sends no \`${field}\``);
  }
  // And the other way: a field the struct does not declare is dropped by serde, silently.
  for (const key of Object.keys(payload)) {
    assert.ok(fields.includes(key), `payloadFor sends \`${key}\`, which NotifyPayload does not take`);
  }
});
