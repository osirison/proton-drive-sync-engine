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
  columnCopy,
  consequenceOf,
  factsOf,
  gateSatisfied,
  itemKey,
  kindNoteOf,
  splitQueue,
} from "../src/js/screens/deletions.js";
import { DELETIONS } from "../src/js/ui/copy.js";
import { severityOf, severityOfItem, splitEmphasis } from "../src/js/ui/rows.js";
import { monthYear, since } from "../src/js/ui/format.js";

const folder = {
  path: "photos/2019",
  direction: "local",
  entity_kind: "directory",
  fingerprint: "vol~node",
  detected_epoch_secs: 1_700_000_000,
  first_seen_epoch_secs: 1_700_000_000,
  subtree_files: 1204,
  subtree_bytes: 8_400_000_000,
};

/** The same folder as a daemon that predates the lifecycle fields reports it (#208/#225). */
const olderFolder = {
  ...folder,
  first_seen_epoch_secs: 0,
  subtree_files: null,
  subtree_bytes: null,
};
const file = {
  path: "archive/old-notes.md",
  direction: "remote",
  entity_kind: "file",
  fingerprint: "0b4f",
  detected_epoch_secs: 1_700_000_500,
  first_seen_epoch_secs: 1_700_000_500,
  subtree_files: null,
  subtree_bytes: null,
};

// ---- which column ---------------------------------------------------------------------------

test("direction names the side the delete lands on, not the side it came from", () => {
  // `Local` = remove it from THIS COMPUTER, because it already went on Proton. No trash: permanent.
  assert.equal(severityOf("local"), "permanent");
  // `Remote` = move PROTON's copy to the Trash, because it already went here. Recoverable.
  assert.equal(severityOf("remote"), "recoverable");
});

test("an unrecognised direction is treated as permanent, not as recoverable", () => {
  // FAIL CLOSED. The recoverable column has no typed gate and its one button approves in a single
  // click, so anything the wire sends that is not exactly `remote` — a missing field, a typo, a
  // third `DeleteDirection` added upstream — must land on the side that makes you type the word.
  for (const direction of [undefined, null, "", "LOCAL", "trash", "Remote"]) {
    assert.equal(severityOf(direction), "permanent", `"${direction}" must not skip the gate`);
  }
  const stray = { path: "x", direction: "who knows", entity_kind: "file", fingerprint: "f" };
  const [permanent, recoverable] = splitQueue([stray]);
  assert.equal(permanent.length, 1);
  assert.deepEqual(recoverable, []);
});

// ---- the disposal, which is what direction stopped answering -----------------------------------

/** The same local deletion as a trash-mode daemon reports it. */
const trashedFile = { ...folder, path: "photos/2019", disposal: "recoverable" };

test("a local deletion the daemon will trash is recoverable, not permanent", () => {
  // The whole GUI half of the change. `direction` still says `local`; the daemon says it can be
  // brought back, and that is what decides.
  assert.equal(severityOfItem(trashedFile), "recoverable");
  const [permanent, recoverable] = splitQueue([trashedFile]);
  assert.deepEqual(permanent, []);
  assert.equal(recoverable.length, 1);
});

test("an absent or unrecognised disposal keeps the gate, in both arguments", () => {
  // FAIL CLOSED ON BOTH. An older daemon sends no `disposal` and really did unlink; a newer one
  // could send a word this build has never heard. Neither may reach the one-click button.
  for (const disposal of [undefined, null, "", "trash", "Recoverable", "shredded", "permanent"]) {
    assert.equal(
      severityOfItem({ ...folder, disposal }),
      "permanent",
      `disposal "${disposal}" must not skip the gate`,
    );
  }
  // `"trash"` in particular: that is the CONFIG spelling and never crosses the wire. Accepting it
  // would mean this function had two ideas about where its input comes from.
  assert.equal(severityOf("local", "trash"), "permanent");
  assert.equal(severityOf("local", "recoverable"), "recoverable");
});

test("a recoverable deletion names the trash it is actually going to", () => {
  // Naming the wrong trash is worse than naming none: a person who goes looking will not find it.
  const remote = consequenceOf(file);
  assert.equal(remote.sentence, DELETIONS.travelExplainer);
  assert.equal(remote.emphasis, "Proton Drive's Trash");

  const local = consequenceOf(trashedFile);
  assert.equal(local.sentence, DELETIONS.travelExplainerLocal);
  assert.equal(local.emphasis, "this computer's Trash");
  assert.ok(
    local.sentence.includes(local.emphasis),
    "the emphasis must be a slice of its own sentence",
  );
});

test("a permanent deletion keeps every word it had", () => {
  // The opt-out draws exactly what it drew before. Nothing about the warnings was deleted — they
  // are conditional now, and this is the condition.
  assert.equal(
    consequenceOf(folder).sentence,
    DELETIONS.folderConsequence("1,204 files, 8.4 GB"),
  );
  assert.equal(consequenceOf({ ...file, direction: "local" }).sentence, DELETIONS.fileConsequence);
  assert.equal(severityOfItem({ ...folder, disposal: "permanent" }), "permanent");
});

test("a column's header names what is in it, not which column it is", () => {
  // The recoverable column used to have one destination because only a remote deletion was
  // recoverable. It can now hold both, and a header keyed on the COLUMN would tell half the cards
  // under it to look in the wrong trash.
  assert.deepEqual(columnCopy("permanent", [folder]), {
    eyebrowText: DELETIONS.permanent,
    note: DELETIONS.permanentSub,
  });
  assert.deepEqual(columnCopy("recoverable", [file]), {
    eyebrowText: DELETIONS.recoverable,
    note: DELETIONS.recoverableSub,
  });
  assert.deepEqual(columnCopy("recoverable", [trashedFile]), {
    eyebrowText: DELETIONS.recoverableLocal,
    note: DELETIONS.recoverableLocalSub,
  });
  // Both under one header: it names no destination, because the two cards have different ones.
  assert.deepEqual(columnCopy("recoverable", [file, trashedFile]), {
    eyebrowText: DELETIONS.recoverableMixed,
    note: DELETIONS.recoverableMixedSub,
  });
});

test("a trash-mode queue empties the permanent column rather than mislabelling it", () => {
  // The commonest shape of the whole feature: a default daemon, only local deletions waiting. The
  // permanent column has nothing in it and is not drawn — the same rule that already hides the
  // recoverable column under `only ask about permanent ones`, not a second one keyed off a config
  // value the screen would have to keep in step with.
  const [permanent, recoverable] = splitQueue([trashedFile, { ...trashedFile, path: "a.txt" }]);
  assert.deepEqual(permanent, []);
  assert.equal(recoverable.length, 2);
  assert.equal(columnCopy("recoverable", recoverable).eyebrowText, DELETIONS.recoverableLocal);
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
  for (const item of [folder, olderFolder, file, { ...folder, entity_kind: "file" }]) {
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

test("a permanent folder names what its subtree costs, and emphasises that figure", () => {
  // #208. FILES where the frame says photos: `subtree_files` counts files and no engine knows they
  // are photographs. The figure is the emphasis, which is voice rule 2 — the loss, in the words a
  // person weighs it in.
  const { sentence, emphasis } = consequenceOf(folder);
  assert.equal(emphasis, "1,204 files, 8.4 GB");
  assert.equal(sentence, DELETIONS.folderConsequence("1,204 files, 8.4 GB"));
});

test("a folder with no countable subtree says everything inside it goes, and names no count", () => {
  // An older daemon, and an EMPTY folder, take the same branch: `Deleting this removes 0 files` is
  // a sentence about nothing, and a fabricated number is what DEVIATIONS §60 forbids outright.
  for (const item of [olderFolder, { ...folder, subtree_files: 0, subtree_bytes: 0 }]) {
    const { sentence } = consequenceOf(item);
    assert.equal(sentence, DELETIONS.folderConsequenceUnknown);
    assert.doesNotMatch(sentence, /\d/, "no figure may be invented for a folder's loss");
  }
});

test("a permanent FILE gets its own sentence — no frame draws one", () => {
  const { sentence } = consequenceOf({ ...folder, entity_kind: "file" });
  assert.equal(sentence, DELETIONS.fileConsequence);
  assert.notEqual(sentence, DELETIONS.folderConsequenceUnknown, "a file has no inside");
});

// ---- the facts strip ------------------------------------------------------------------------

test("a folder card draws when it was deleted, and never the atime beside it", () => {
  // #225 gave the strip its first fact — the first-seen time, which survives passes and restarts.
  // The second is still `last opened`, an atime the index does not store (#208), so a folder's
  // strip is one fact and the mtime is not substituted for it: a directory record's mtime is the
  // directory's own, which is not when anything in it was edited.
  assert.deepEqual(factsOf(folder, undefined), [
    { at: 0, text: DELETIONS.deletedOnProton(since(folder.first_seen_epoch_secs, "short")) },
  ]);
  assert.deepEqual(factsOf(folder, { mtime: 1_767_000_000 }), factsOf(folder, undefined));
});

test("a zero first-seen is unknown, not 1970", () => {
  // `#[serde(default)]` on the wire: a daemon that predates the field sends `0`, and ageing from
  // the epoch would draw `55y ago` on a deletion from this morning.
  assert.deepEqual(factsOf(olderFolder, undefined), []);
  assert.deepEqual(factsOf({ ...file, first_seen_epoch_secs: 0 }, { mtime: 1_767_000_000 }), [
    { at: 1, text: DELETIONS.lastEdited(monthYear(1_767_000_000)) },
  ]);
});

test("a file draws its mtime, and nothing when no record arrived", () => {
  // `at` is the DRAWN slot, not the DOM position: Phase 1 omits the frame's first fact, so its one
  // fact stands for `span[1]`. Stamped by position it would be compared against `deleted here 6m
  // ago` — a different node — and reported as a width failure on a correct card.
  const deleted = { at: 0, text: DELETIONS.deletedHere(since(file.first_seen_epoch_secs, "short")) };
  assert.deepEqual(factsOf(file, { mtime: 1_767_000_000, file_size: 4096 }), [
    deleted,
    { at: 1, text: DELETIONS.lastEdited(monthYear(1_767_000_000)) },
  ]);
  // The reply lands a render after the screen does, and may never land at all.
  assert.deepEqual(factsOf(file, undefined), [deleted]);
  assert.deepEqual(factsOf(file, { mtime: null }), [deleted]);
});

test("the age is the FIRST-SEEN time and never the pass's own", () => {
  // The whole of #225 in one assertion. `detected_epoch_secs` is re-stamped every pass — the two
  // fields are minutes apart here, and only one of them may reach the card.
  const stale = { ...folder, detected_epoch_secs: 1_900_000_000 };
  assert.deepEqual(factsOf(stale, undefined), factsOf(folder, undefined));
});

test("the wording follows the column, so each side names the other one", () => {
  // Permanent = it went on Proton first; recoverable = it went here first. Reading this backwards
  // tells the user the deletion happened on the side that still has the file.
  assert.match(factsOf(folder, undefined)[0].text, /^deleted on Proton /);
  assert.match(factsOf(file, undefined)[0].text, /^deleted here /);
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
