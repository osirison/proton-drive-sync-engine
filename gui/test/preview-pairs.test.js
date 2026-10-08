// The one-pair injection `?pairs=1` applies (#102 phase 5a-2, K8/L2).
//
// `check-n1-identity.mjs` compares a frame rendered as authored with the same frame answered as a
// current daemon running one pair would answer it. That comparison is only a claim if the injection
// REACHED the reply, and it passed at 51/51 while `withOnePairConfig` reached none: it returned any
// config whose `pairs` was an array untouched, an empty array included, and every fixture's is
// empty. The gate cannot see that, because a reply compared with itself is equal. So the decision is
// pinned here, on each config shape, where a revert fails by name.

import { test } from "node:test";
import assert from "node:assert/strict";
import { withOnePair, withOnePairConfig } from "../src/js/fixtures/preview.js";
import { DEFAULT_PAIR } from "../src/js/store.js";

test("a config that lists no pairs gains the one implicit pair, carrying the file's roots", () => {
  const config = { exists: true, local_root: "/home/u/Sync", remote_root: "/Drive/Sync", pairs: [] };
  const out = withOnePairConfig(config);
  assert.notEqual(out, config, "the empty list is rewritten, not handed back");
  assert.deepEqual(out.pairs, [
    { name: DEFAULT_PAIR, local_root: "/home/u/Sync", remote_root: "/Drive/Sync" },
  ]);
  assert.equal(out.exists, true, "everything else is kept");
  assert.deepEqual(config.pairs, [], "the fixture itself is not mutated");
});

test("a config with no `pairs` key at all gains the pair, and a missing file places no roots", () => {
  const out = withOnePairConfig({ exists: false, toml: "" });
  assert.deepEqual(out.pairs, [{ name: DEFAULT_PAIR, local_root: null, remote_root: null }]);
});

test("a config that already lists a pair is returned as it is", () => {
  const config = {
    exists: true,
    pairs: [{ name: "work", local_root: "/w", remote_root: "/Drive/w" }],
  };
  assert.equal(withOnePairConfig(config), config, "the same object: nothing was decided for it");
});

test("no config at all is passed through", () => {
  assert.equal(withOnePairConfig(undefined), undefined);
  assert.equal(withOnePairConfig(null), null);
});

test("a status with a reply gains `pair`/`pairs` on the reply and `selected`/`pairs`/`pair_states` on the payload", () => {
  const status = { state: "idle", response: { paused: false, syncing: false, reconcile_seq: 3 } };
  const out = withOnePair(status);
  assert.notEqual(out.response, status.response);
  assert.equal(out.response.pair, DEFAULT_PAIR);
  assert.equal(out.response.pairs.length, 1);
  assert.equal(out.selected, DEFAULT_PAIR);
  assert.equal(out.pairs.length, 1);
  assert.deepEqual(out.pair_states, [{ name: DEFAULT_PAIR, state: "idle" }]);
  assert.equal(status.response.pair, undefined, "the fixture itself is not mutated");
});

test("a status whose socket failed gains `selected` alone, and no status stays none", () => {
  const status = { state: "unreachable" };
  const out = withOnePair(status);
  assert.deepEqual(out, { state: "unreachable", selected: DEFAULT_PAIR });
  assert.equal(withOnePair(undefined), undefined);
});
