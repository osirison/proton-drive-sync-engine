// A folder pair may be named anything within `[A-Za-z0-9._-]{1,64}` (#102 phase 5c-1, carried from the
// review of #445), and that includes the names a plain object already answers: `constructor`,
// `toString`, `hasOwnProperty`, `valueOf`, `isPrototypeOf` and `__proto__`. Every table in the GUI that
// is keyed by a pair's name is a `Map` made by `pairmap.js`; these tests hold that from three sides —
// the helper itself, the store that files a reply under the pair it describes, and the sources that
// would have to be edited to put an object back.

import { test } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";
import * as espree from "espree";
import { pairTable, withPair } from "../src/js/pairmap.js";
import * as store from "../src/js/store.js";

/** Names a plain object answers for. Each is a legal pair name. */
const AWKWARD = ["constructor", "toString", "hasOwnProperty", "valueOf", "isPrototypeOf", "__proto__"];

const source = (path) => readFileSync(fileURLToPath(new URL(path, import.meta.url)), "utf8");

/**
 * The code of a module with its comments (and whitespace) removed: its tokens, joined by one space.
 * The sources describe the very thing they must not do in their own comments (`byPair[name]` is in
 * store.js's header), so a scan of the raw text accuses the explanation.
 */
const code = (path) =>
  espree
    .tokenize(source(path), { ecmaVersion: 2023, sourceType: "module" })
    .map((token) => token.value)
    .join(" ");

test("a_table_keyed_by_pair_name_answers_only_for_pairs_it_was_given", () => {
  for (const name of AWKWARD) {
    const empty = pairTable();
    assert.equal(empty.get(name), undefined, `${name} is not in an empty table`);
    assert.equal(empty.has(name), false);

    const filled = withPair(empty, name, { mine: true });
    assert.deepEqual(filled.get(name), { mine: true }, `${name} reads back what was filed under it`);
    assert.equal(empty.has(name), false, "withPair copies — the table it was given is untouched");
    // Nothing leaked onto the prototype chain of every object in the page.
    assert.equal({}.mine, undefined, `${name} wrote onto Object.prototype`);
    assert.equal(Object.getPrototypeOf({}), Object.prototype);
    // …and another pair's slot did not move.
    assert.equal(filled.get("documents"), undefined);
  }
});

test("the_store_files_and_reads_a_reply_under_a_pair_with_an_awkward_name", () => {
  let issue = 0;
  const summary = (name) => ({
    name,
    local_root: `/l/${name}`,
    remote_root: `/Drive/${name}`,
    db_path: `/l/${name}/.sync/sync_index.db`,
    paused: false,
    syncing: false,
    reconcile_seq: 0,
    last_sync_epoch_secs: 1,
    last_error: null,
    pending_changes: 0,
    pending_deletions: 0,
  });
  for (const name of AWKWARD) {
    store.configure({ follows: "selection" });
    const pairs = ["documents", name].map(summary);
    issue += 1;
    store.setStatus(
      {
        state: "running",
        selected: name,
        pairs,
        pair_states: pairs.map((p) => ({ name: p.name, state: "idle" })),
        response: {
          pair: name,
          pairs,
          syncing: true,
          pending_changes: 7,
          pending_deletions: [{ path: "a.txt", direction: "remote", fingerprint: "f" }],
        },
      },
      issue,
    );
    assert.equal(store.select.pairName(), name, `${name} is the selected pair`);
    assert.equal(
      store.select.daemonState(),
      "running",
      `${name}: the state is the reply's, not a function's`,
    );
    assert.equal(store.select.response().pending_changes, 7);
    assert.deepEqual(
      store.select.pendingDeletions().map((d) => [d.path, d.pair]),
      [["a.txt", name]],
      `${name}: a deletion is tagged with the pair it was filed under`,
    );

    store.setConflicts([{ original: "note.txt", sidecar: "note.proton-cloud.txt" }], name);
    assert.deepEqual(
      store.select.conflicts().map((c) => c.pair),
      [name],
    );
    store.stageResolution("note.txt", "keep_both", name);
    assert.equal(store.select.stagedWriteCount(), 1);
    // Another folder of the same session is not touched by any of it.
    assert.equal({}.constructor, Object, "Object.constructor is still Object");
  }
});

test("no_per_pair_table_in_the_sources_is_a_plain_object", () => {
  const app = code("../src/js/app.js");
  const storeSource = code("../src/js/store.js");

  // The two tables app.js keys by pair, and the one the store does, are made by the helper.
  assert.match(app, /let configByPair = pairTable \( \) ;/);
  assert.match(app, /let settingsByPair = pairTable \( \) ;/);
  assert.match(storeSource, /byPair : pairTable \( \) ,/);

  // And none of the three is ever indexed like an object or reset to a literal.
  for (const [file, text] of [
    ["app.js", app],
    ["store.js", storeSource],
  ]) {
    assert.doesNotMatch(text, /\b\w*[bB]yPair = \{/, `${file} assigns an object literal to a per-pair table`);
    assert.doesNotMatch(text, /\b\w*[bB]yPair : \{/, `${file} declares a per-pair table as a literal`);
    assert.doesNotMatch(text, /\b\w*[bB]yPair \[/, `${file} indexes a per-pair table like an object`);
    assert.doesNotMatch(text, /\.\.\. \w*[bB]yPair\b/, `${file} spreads a per-pair table into an object`);
  }
});
