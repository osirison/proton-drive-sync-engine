// The command facade's pair rules (#102 phase 5a-2, brief section 2.3).
//
// Two classes, decided by what a wrong folder pair costs. A READ may name a pair or leave it to the
// selection Rust holds; a WRITE must name the one its caller captured, and a missing name is a
// rejected call with nothing sent — never "the default pair" and never "the selected one", because a
// silent default is the wrong-folder bug with its error message removed.
//
// Driven through the real `api` object against a recording stand-in for Tauri's `invoke`, so what is
// asserted is what would have crossed the bridge.

import { test, beforeEach, afterEach } from "node:test";
import assert from "node:assert/strict";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

const calls = [];
beforeEach(() => {
  calls.length = 0;
  globalThis.window = {
    __TAURI__: {
      core: {
        invoke: (cmd, args) => {
          calls.push({ cmd, args });
          return Promise.resolve(null);
        },
      },
    },
  };
});
afterEach(() => {
  delete globalThis.window;
});

const { api } = await import("../src/js/api.js");

const CONFLICT = {
  original: "a.txt",
  sidecar: "a.proton-cloud.txt",
  kind: "content",
  pair: "tagged-by-the-store",
};

/** Every class-W wrapper, called with a pair and without one. */
const WRITES = [
  ["pause", "pause", (opts) => api.pause(opts), {}],
  ["resume", "resume", (opts) => api.resume(opts), {}],
  ["sync_now", "syncNow", (opts) => api.syncNow(opts), {}],
  ["resync", "resync", (opts) => api.resync(opts), {}],
  [
    "approve",
    "approve",
    (opts) => api.approve("a.txt", true, "local", opts),
    { target: "a.txt", literalPath: true, direction: "local" },
  ],
  ["deny", "deny", (opts) => api.deny("a.txt", true, opts), { target: "a.txt", literalPath: true }],
  ["keep", "keep", (opts) => api.keep("a.txt", false, opts), { target: "a.txt", literalPath: false }],
  ["run_dry_run", "runDryRun", (opts) => api.runDryRun(opts), {}],
  [
    "apply_plan",
    "applyPlan",
    (opts) => api.applyPlan("tok", true, opts),
    { token: "tok", skipDestructive: true },
  ],
  [
    "resolve_conflict",
    "resolveConflict",
    (opts) => api.resolveConflict(CONFLICT, "keep_both", opts),
    { conflict: { original: "a.txt", sidecar: "a.proton-cloud.txt", kind: "content" }, choice: "keep_both" },
  ],
  // THE SETTINGS SAVE (#102 phase 5b-1). A write to ONE pair's table, so the pair is the one the staged
  // edits belong to — and it is sent beside the update, not inside it, because the update is the
  // file's keys and the pair is which table of the file they go to.
  [
    "write_config",
    "writeConfig",
    (opts) => api.writeConfig({ scan_interval_secs: 60, log_level: "debug" }, opts),
    { update: { scan_interval_secs: 60, log_level: "debug" } },
  ],
];

test("a_class_w_wrapper_without_a_pair_rejects_and_sends_nothing", async () => {
  for (const [command, name, call] of WRITES) {
    await assert.rejects(() => call(undefined), /a folder pair is required/, name);
    await assert.rejects(() => call({}), /a folder pair is required/, `${name} with an empty options object`);
    await assert.rejects(() => call({ pair: "" }), /a folder pair is required/, `${name} with an empty name`);
    await assert.rejects(() => call({ pair: null }), /a folder pair is required/, `${name} with a null name`);
    assert.deepEqual(calls, [], `${command}: a refused call must not reach the bridge`);
  }
});

test("a_class_w_wrapper_sends_exactly_its_arguments_and_the_pair_it_was_given", async () => {
  for (const [command, name, call, args] of WRITES) {
    calls.length = 0;
    await call({ pair: "photos" });
    assert.equal(calls.length, 1, name);
    assert.equal(calls[0].cmd, command, name);
    assert.deepEqual(
      calls[0].args,
      { ...args, pair: "photos" },
      `${name}: the pair travels with the arguments`,
    );
  }
});

test("the_conflict_the_store_tagged_is_sent_without_the_tag", async () => {
  // The wire's `Conflict` has three fields; the store's `pair` is for the caller.
  await api.readConflictPair(CONFLICT, { pair: "docs" });
  await api.resolveConflict(CONFLICT, "use_proton", { pair: "docs" });
  for (const call of calls) {
    assert.deepEqual(call.args.conflict, {
      original: "a.txt",
      sidecar: "a.proton-cloud.txt",
      kind: "content",
    });
    assert.equal(call.args.pair, "docs");
  }
});

test("a_class_r_wrapper_with_no_pair_sends_what_it_always_sent", async () => {
  // The selection is Rust's: with nothing named, the call is byte for byte the one this app made
  // before there were pairs — the half of "a one-folder user sees nothing new" that crosses the bridge.
  await api.getStatus();
  await api.listPendingDeletions();
  await api.scanConflicts();
  await api.readConflictPair(CONFLICT);
  await api.pathSyncStatus("a.txt");
  await api.searchFiles("a", 5);
  await api.openPaths(["a.txt"]);
  await api.openFolder("a.txt");
  await api.freeSpace(null);
  await api.skipRuleUsage(["*.tmp"], []);
  await api.readConfig();
  assert.deepEqual(
    calls.map((call) => [call.cmd, call.args]),
    [
      ["get_status", undefined],
      ["list_pending_deletions", undefined],
      ["scan_conflicts", undefined],
      [
        "read_conflict_pair",
        { conflict: { original: "a.txt", sidecar: "a.proton-cloud.txt", kind: "content" } },
      ],
      ["path_sync_status", { relativePath: "a.txt" }],
      ["search_files", { query: "a", limit: 5 }],
      ["open_paths", { relative: ["a.txt"] }],
      ["open_folder", { relative: "a.txt" }],
      ["free_space", { path: null }],
      ["skip_rule_usage", { patterns: ["*.tmp"], include: [] }],
      ["read_config", undefined],
    ],
  );
});

test("a_class_r_wrapper_may_name_a_pair_and_then_does", async () => {
  await api.getStatus({ pair: "photos" });
  await api.scanConflicts({ pair: "photos" });
  await api.pathSyncStatus("a.txt", { pair: "photos" });
  await api.searchFiles("a", undefined, { pair: "photos" });
  await api.openFolder("a.txt", { pair: "photos" });
  await api.readConfig({ pair: "photos" });
  for (const call of calls) assert.equal(call.args.pair, "photos", call.cmd);
  // A name left undefined (the tray panel before the daemon has listed its pairs) names nothing.
  calls.length = 0;
  await api.getStatus({ pair: undefined });
  assert.deepEqual(calls, [{ cmd: "get_status", args: undefined }]);
});

test("select_pair_sends_the_name", async () => {
  await api.selectPair("photos");
  assert.deepEqual(calls, [{ cmd: "select_pair", args: { name: "photos" } }]);
});

test("every_class_w_command_the_backend_names_goes_through_the_requiring_wrapper", () => {
  // Read the Rust table of classes so the two cannot drift: a command added there as W and wired here
  // through plain `invoke` would take no pair, and the facade would be sending a write the backend
  // refuses (or, worse, once it did not refuse, a write nobody had to name a pair for).
  const rust = readFileSync(
    fileURLToPath(new URL("../src-tauri/src/commands/pair_tests.rs", import.meta.url)),
    "utf8",
  );
  const writes = [...rust.matchAll(/\("(\w+)", Class::W\)/g)].map((m) => m[1]);
  assert.ok(writes.length >= 10, `found ${writes.length} class-W commands in pair_tests.rs`);
  const source = readFileSync(fileURLToPath(new URL("../src/js/api.js", import.meta.url)), "utf8");
  for (const command of writes) {
    assert.ok(source.includes(`write("${command}"`), `api.js must send ${command} through write()`);
    assert.ok(!source.includes(`invoke("${command}"`), `api.js sends ${command} without requiring a pair`);
  }
  assert.deepEqual(
    [...writes].sort(),
    WRITES.map(([command]) => command).sort(),
    "this test's table of writes is not the backend's",
  );
});
