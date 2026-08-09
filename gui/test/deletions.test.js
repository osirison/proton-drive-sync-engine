// The deletions screen's pure decisions (S3), tested on their own because a passing style gate
// would not defend any of them: every one is about a state no `4a` frame draws.
//
// The screen is a queue, and a frame is one arrangement of one queue. What the gate checks is two
// items, one per column, one of them armed. What it cannot check is a second card in a column, an
// empty column, an armed path naming something that is no longer waiting, a permanent FILE (both
// drawn permanent items are folders), or a file whose index record never arrived. Those are where
// this screen goes wrong, so those are what is here.
//
// The one that would fail loudest and quietest at once is `severityOf`. It maps the wire's
// `direction` onto the column an item is drawn in, and reading it backwards produces a complete,
// plausible screen that offers `Move to Proton's Trash` for a file about to leave the disk for
// good — no error, no visual glitch, and the safest-looking half of the screen doing the most
// dangerous thing.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  GATE_WORD,
  armedItem,
  bodyOf,
  consequenceOf,
  factsOf,
  gateSatisfied,
  itemKey,
  kindNoteOf,
  splitQueue,
} from "../src/js/screens/deletions.js";
import { DELETIONS } from "../src/js/ui/copy.js";
import { severityOf, splitEmphasis } from "../src/js/ui/rows.js";
import { monthYear } from "../src/js/ui/format.js";

const folder = {
  path: "photos/2019",
  direction: "local",
  entity_kind: "directory",
  fingerprint: "vol~node",
  detected_epoch_secs: 1_700_000_000,
};
const file = {
  path: "archive/old-notes.md",
  direction: "remote",
  entity_kind: "file",
  fingerprint: "0b4f",
  detected_epoch_secs: 1_700_000_500,
};

// ---- which column ---------------------------------------------------------------------------

test("direction names the side the delete lands on, not the side it came from", () => {
  // `Local` = remove it from THIS COMPUTER, because it already went on Proton. No trash: permanent.
  assert.equal(severityOf("local"), "permanent");
  // `Remote` = move PROTON's copy to the Trash, because it already went here. Recoverable.
  assert.equal(severityOf("remote"), "recoverable");
});

test("the queue splits into the drawn order — permanent first, recoverable second", () => {
  const [permanent, recoverable] = splitQueue([file, folder]);
  assert.deepEqual(
    permanent.map((i) => i.path),
    ["photos/2019"],
  );
  assert.deepEqual(
    recoverable.map((i) => i.path),
    ["archive/old-notes.md"],
  );
});

test("a column with nothing in it comes back empty, which is how the policy is honoured", () => {
  // `only ask about permanent ones` means the daemon never withholds a Proton-side delete, so the
  // recoverable column has no items — the screen needs no second rule for the setting.
  const [permanent, recoverable] = splitQueue([folder]);
  assert.equal(permanent.length, 1);
  assert.deepEqual(recoverable, []);
});

test("more than one card in a column, which no frame draws", () => {
  const second = { ...folder, path: "photos/2020" };
  const [permanent] = splitQueue([folder, second]);
  assert.equal(permanent.length, 2, "the second permanent deletion must not be dropped");
});

// ---- which body -----------------------------------------------------------------------------

test("an empty queue is the empty state whatever else is set", () => {
  assert.equal(bodyOf({ items: [], armed: null }), "empty");
  // EMPTY OUTRANKS ARMED. The pass ran while the confirmation was up, so the deletion it is asking
  // about is already gone — confirming it would be answering a question with no subject.
  assert.equal(bodyOf({ items: [], armed: { path: "photos/2019", fingerprint: "vol~node" } }), "empty");
});

const armedAt = (item) => ({ path: item.path, fingerprint: item.fingerprint });

test("armed only when the queue still holds the item it names", () => {
  assert.equal(bodyOf({ items: [folder, file], armed: armedAt(folder) }), "armed");
  assert.equal(bodyOf({ items: [folder, file], armed: null }), "queue");
  // A path that has left the queue — settled by another client, or applied by a pass.
  const gone = { path: "photos/2018", fingerprint: "x" };
  assert.equal(bodyOf({ items: [folder, file], armed: gone }), "queue");
});

test("a recoverable item can never be armed", () => {
  // The typed gate exists on the permanent column alone; arming a Trash move would put the app's
  // only solid-red confirmation in front of a reversible action.
  assert.equal(bodyOf({ items: [file], armed: armedAt(file) }), "queue");
  assert.equal(armedItem([file], armedAt(file)), null);
});

test("THE SAME PATH IS NOT THE SAME DELETION — the takeover does not re-bind by path", () => {
  // Arm `photos/2019`; that deletion resolves; the folder comes back and is deleted again. Matching
  // on the path alone would re-open the full-window confirmation on the NEW deletion, with a live
  // `Delete permanently` that no word was typed for.
  const reborn = { ...folder, fingerprint: "vol~node-2" };
  assert.equal(armedItem([reborn], armedAt(folder)), null);
  assert.equal(bodyOf({ items: [reborn], armed: armedAt(folder) }), "queue");
  assert.equal(armedItem([reborn], armedAt(reborn)), reborn);
});

test("armedItem returns the item itself, so the takeover cannot be built from a stale path", () => {
  assert.equal(armedItem([folder, file], armedAt(folder)), folder);
  assert.equal(armedItem([folder], null), null);
  assert.equal(armedItem([], armedAt(folder)), null);
});

// ---- the gate -------------------------------------------------------------------------------

test("the typed word is case-sensitive and exact", () => {
  assert.equal(gateSatisfied("DELETE"), true);
  // `delete` is a word people type by habit; `DELETE` is not. That is the whole gate.
  assert.equal(gateSatisfied("delete"), false);
  assert.equal(gateSatisfied("Delete"), false);
  assert.equal(gateSatisfied(" DELETE"), false);
  assert.equal(gateSatisfied("DELETE "), false);
  assert.equal(gateSatisfied(""), false);
  assert.equal(gateSatisfied(undefined), false);
});

test("the word the field checks is the word the hint tells you to type", () => {
  assert.ok(DELETIONS.typeToDelete.includes(GATE_WORD));
});

// ---- identity -------------------------------------------------------------------------------

test("an item is keyed by path AND direction, as the daemon's own approvals table is", () => {
  assert.notEqual(itemKey(folder), itemKey({ ...folder, direction: "remote" }));
  assert.equal(itemKey(folder), itemKey({ ...folder, fingerprint: "different" }));
});

// ---- what the card says ---------------------------------------------------------------------

test("every consequence emphasises a substring of its own sentence", () => {
  // The contract `splitEmphasis` falls back on. A sentence and an emphasis chosen in two places
  // drift; chosen together they cannot, and this is what says so.
  for (const item of [folder, file, { ...folder, entity_kind: "file" }]) {
    const { sentence, emphasis } = consequenceOf(item);
    assert.ok(sentence.includes(emphasis), `"${emphasis}" is not in "${sentence}"`);
    assert.equal(splitEmphasis(sentence, emphasis).length, 3, "the split must find it");
  }
});

test("a recoverable deletion says where the file goes, not what is lost", () => {
  const { sentence, emphasis } = consequenceOf(file);
  assert.equal(sentence, DELETIONS.travelExplainer);
  assert.equal(emphasis, "Proton Drive's Trash");
});

test("a permanent folder says everything inside it goes, and names no count", () => {
  const { sentence } = consequenceOf(folder);
  assert.equal(sentence, DELETIONS.folderConsequenceUnknown);
  // #208. The drawn sentence's `1,204 photos, 8.4 GB` is a subtree aggregate no command produces,
  // and a fabricated number on this card is the one thing DEVIATIONS §60 forbids outright.
  assert.doesNotMatch(sentence, /\d/, "Phase 1 may not put a figure on a folder's loss");
});

test("a permanent FILE gets its own sentence — no frame draws one", () => {
  const { sentence } = consequenceOf({ ...folder, entity_kind: "file" });
  assert.equal(sentence, DELETIONS.fileConsequence);
  assert.notEqual(sentence, DELETIONS.folderConsequenceUnknown, "a file has no inside");
});

// ---- the facts strip ------------------------------------------------------------------------

test("a folder card draws no facts at all, because both of the frame's are unavailable", () => {
  // #225 — `detected_epoch_secs` is re-stamped on every pass, so `deleted on Proton 22m ago` is the
  // age of the pass rather than of the deletion. #208 — `last opened` is an atime. Two omissions
  // leave nothing, and nothing is what is drawn: an em-dash would claim the daemon was asked.
  assert.deepEqual(factsOf(folder, undefined), []);
  assert.deepEqual(factsOf(folder, { mtime: 1_767_000_000 }), []);
});

test("a file draws its mtime, and nothing when no record arrived", () => {
  // `at` is the DRAWN slot, not the DOM position: Phase 1 omits the frame's first fact, so its one
  // fact stands for `span[1]`. Stamped by position it would be compared against `deleted here 6m
  // ago` — a different node — and reported as a width failure on a correct card.
  assert.deepEqual(factsOf(file, { mtime: 1_767_000_000, file_size: 4096 }), [
    { at: 1, text: DELETIONS.lastEdited(monthYear(1_767_000_000)) },
  ]);
  // The reply lands a render after the screen does, and may never land at all.
  assert.deepEqual(factsOf(file, undefined), []);
  assert.deepEqual(factsOf(file, { mtime: null }), []);
});

test("no fact is rendered from the detected time, in either direction", () => {
  // The deck still carries both sentences — the frames draw them and the copy gate checks them —
  // and nothing on the screen may reach for them until #225 lands.
  for (const item of [folder, file]) {
    for (const { text } of factsOf(item, { mtime: 1_767_000_000 })) {
      assert.ok(!text.startsWith("deleted on Proton"), text);
      assert.ok(!text.startsWith("deleted here"), text);
    }
  }
});

// ---- the kind note --------------------------------------------------------------------------

test("a folder says what it is; a file says how big it is", () => {
  assert.equal(kindNoteOf(folder, undefined), "a folder");
  assert.equal(kindNoteOf(file, { file_size: 4096 }), "4 KB");
});

test("a file with no record draws nothing rather than an em-dash", () => {
  // A dash in the size slot of a card about losing a file reads as "zero bytes", which is a claim.
  assert.equal(kindNoteOf(file, undefined), null);
  assert.equal(kindNoteOf(file, { file_size: null }), null);
});

// ---- the copy that counts ---------------------------------------------------------------------

test("one waiting deletion is not `One files are waiting`", () => {
  assert.equal(DELETIONS.title(2), "Two files are waiting to be deleted");
  assert.equal(DELETIONS.title(1), "One file is waiting to be deleted");
  assert.equal(DELETIONS.compact.title(1), "1 file waiting to be deleted");
  assert.equal(DELETIONS.compact.title(2), "2 files waiting to be deleted");
});

test("the armed sentence drops its size clause rather than dashing it", () => {
  const withSize = DELETIONS.armedBody("photos/2019", "8.4 GB");
  const without = DELETIONS.armedBody("photos/2019", null);
  assert.ok(withSize.startsWith("Everything in photos/2019 — 8.4 GB — is removed from disk."));
  assert.ok(without.startsWith("Everything in photos/2019 is removed from disk."));
  assert.doesNotMatch(without, /—/, "an em-dash here would be an answer the daemon never gave");
  // Both name the path, which is what the screen lifts into mono.
  for (const sentence of [withSize, without]) assert.ok(sentence.includes("photos/2019"));
});

test("a permanent FILE gets file grammar in the confirmation, not a folder's", () => {
  // `Everything in archive/old-notes.md` is the frame's sentence applied to a thing with no inside,
  // and no frame draws a permanent file — so nothing but this decides it.
  assert.ok(DELETIONS.armedBodyFile("notes.txt").startsWith("notes.txt is removed from disk."));
  assert.ok(!DELETIONS.armedBodyFile("notes.txt").includes("Everything in"));
  // The two share a tail, so the promise about what cannot be undone is the same either way.
  const tail = "It does not go to your trash";
  assert.ok(DELETIONS.armedBodyFile("a").includes(tail));
  assert.ok(DELETIONS.armedBody("a", null).includes(tail));
});

test("the path's slot in the armed sentence is found by the template, not by searching for it", () => {
  // `indexOf(path)` finds the FIRST textual match, and the sentence has words before the slot: a
  // folder named `in` matched inside `Everything`, and the screen wrapped two letters of the first
  // word in mono. Rendering the template around a marker asks the deck where its own hole is.
  const MARKER = "\u0001";
  for (const render of [(p) => DELETIONS.armedBody(p, null), DELETIONS.armedBodyFile]) {
    const at = render(MARKER).indexOf(MARKER);
    assert.ok(at >= 0, "the template must interpolate its argument");
    for (const path of ["in", "e", "very", "thin", "Everything", "photos/2019", "a b/c d.md"]) {
      assert.equal(render(path).slice(at, at + path.length), path, `${path} landed at the wrong offset`);
    }
  }
});

test("monthYear is a month and a year, and an em-dash for nothing", () => {
  assert.match(monthYear(1_767_000_000), /^[A-Z][a-z]{2} \d{4}$/);
  assert.equal(monthYear(null), "—");
  assert.equal(monthYear(undefined), "—");
  assert.equal(monthYear("nonsense"), "—");
});
