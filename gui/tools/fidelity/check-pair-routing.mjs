// The wiring gate for "a write acts on the pair it was drawn for" (#102 phase 5a-2, acceptance 5 and 6).
//
// `api.test.js` proves the facade refuses a write with no pair, `pair-ledger.test.js` proves the
// state that describes one pair is dropped on a switch, and `selection_tests.rs` proves Rust's side.
// None of them runs `app.js`, and `app.js` is where the pair is CAPTURED: which row a button belongs
// to, which pair a plan was made for, what a press commits to. It cannot be imported (it boots on
// load), so this drives the real page against a scripted stand-in for Tauri's bridge — a Node object
// that answers each command and **holds a reply open when told to**, which is what makes the window
// between "the person pressed it" and "the daemon answered" a thing a test can stand in.
//
// Four scenarios, each a way a write can land on a different pair than the one it was drawn for. Each
// was a bug before the capture existed or would be again if it were removed.
//
//   1. THE TAIL OF A DECISION. `Move to Proton's Trash` approves, waits for the daemon, then nudges a
//      sync. The selection moves while the approval is in flight; the nudge must still go where the
//      approval went.
//   2. THE PRESS OF `Run this sync`. It sends the plan's deletion approvals one at a time and then
//      applies by token. A switch part-way resets the plan screen; the apply must still be the plan
//      that was reviewed, for the pair it was reviewed for.
//   3. A SWITCH IS A CHANGE OF SHAPE. Two folders withhold the same path at the same fingerprint, so
//      the two queues draw identical cards; if the screen patched instead of rebuilding, the new
//      folder's cards would carry the old folder's handlers. `Keep` on the card now showing must go
//      to the folder now showing.
//   4. THE HERO'S BUTTONS. Two folders in the same state draw the same hero, so a switch patches the
//      screen instead of rebuilding it, and its buttons are the ones built for the previous folder.
//      `Pause` must still pause the folder the hero is now about.
//
// WHAT IT CANNOT SEE: it scripts the bridge, so it proves the facade and the screens agree with each
// other, not that the real Rust agrees with either (that is `selection_tests.rs`); and it drives the
// window, not the tray panel.

import puppeteer from "puppeteer";
import { serve } from "./serve.mjs";
import { DELETIONS, MAIN, PLAN } from "../../src/js/ui/copy.js";
import { EMPTY_CONFIG } from "../../src/js/api.js";

const PAIRS = ["docs", "photos"];

const summaryOf = (name, pending) => ({
  name,
  local_root: `/home/u/${name}`,
  remote_root: `/Drive/${name}`,
  db_path: `/home/u/${name}/.sync/sync_index.db`,
  paused: false,
  syncing: false,
  reconcile_seq: 1,
  last_sync_epoch_secs: 1_750_000_000,
  last_error: null,
  pending_changes: 0,
  pending_deletions: pending,
});

const deletion = (path) => ({
  path,
  direction: "remote",
  entity_kind: "file",
  fingerprint: `fp-${path}`,
  detected_epoch_secs: 1_750_000_000,
  first_seen_epoch_secs: 1_750_000_000,
});

const delay = (ms) =>
  new Promise((resolve) => {
    setTimeout(resolve, ms);
  });

/** Poll `check` until it is truthy; reject with `what` if it never is. */
async function until(what, check, ms = 6000) {
  const stop = Date.now() + ms;
  for (;;) {
    const value = await check();
    if (value) return value;
    if (Date.now() > stop) throw new Error(`timed out waiting for ${what}`);
    await delay(25);
  }
}

/** A scripted daemon: answers every command the window sends, records them, and holds replies on request. */
class Bridge {
  constructor(queues) {
    this.calls = [];
    this.selected = "docs";
    this.queues = queues; // pair -> pending deletions
    this.holds = new Map(); // command -> { release }
    this.planFor = (pair) => ({
      report: {
        summary: { total: 1, remote_deletes: 1, destructive_actions: 1 },
        plan: [{ action: "remote_delete", path: `${pair}/gone.txt`, entity_kind: "file" }],
        cannot_sync: [],
      },
      requires_delete_gate: true,
      files_at_risk: [`${pair}/gone.txt`],
      token: `token-for-${pair}`,
      local_disposal: "recoverable",
    });
  }

  status() {
    const name = this.selected;
    const pairs = PAIRS.map((pair) => summaryOf(pair, this.queues[pair].length));
    return {
      state: "idle",
      selected: name,
      pairs,
      pair_states: PAIRS.map((pair) => ({ name: pair, state: "idle" })),
      response: {
        status: "running",
        paused: false,
        syncing: false,
        reconcile_seq: 1,
        pending_changes: 0,
        message: "",
        last_sync_epoch_secs: 1_750_000_000,
        last_error: null,
        last_plan_summary: null,
        last_successful_sync_summary: null,
        status_history: [],
        pending_deletions: this.queues[name],
        failed_items: [],
        failed_item_count: 0,
        config: { local_root: `/home/u/${name}`, remote_root: `/Drive/${name}`, db_path: "/x/i.db" },
        activity: null,
        unsyncable: [],
        auth: "signed-in",
        pair: name,
        pairs,
      },
    };
  }

  /** Hold the next reply to `command` until `release()` is called. */
  hold(command) {
    let release;
    const released = new Promise((resolve) => {
      release = resolve;
    });
    this.holds.set(command, { released, release });
    return release;
  }

  called(command) {
    return this.calls.filter((call) => call.cmd === command);
  }

  async handle(cmd, args) {
    this.calls.push({ cmd, args, selectedThen: this.selected });
    const held = this.holds.get(cmd);
    if (held) {
      this.holds.delete(cmd);
      await held.released;
    }
    switch (cmd) {
      case "get_status":
        return this.status();
      case "read_config":
        return {
          ...EMPTY_CONFIG,
          exists: true,
          pairs: PAIRS.map((name) => ({
            name,
            local_root: `/home/u/${name}`,
            remote_root: `/Drive/${name}`,
          })),
        };
      case "read_notify_policy":
        return "never";
      case "check_cli":
        return { installed: true, distro: null };
      case "scan_conflicts":
        return [];
      case "path_sync_status":
        return { tracked: false };
      case "approve":
        return {
          state: "idle",
          error: null,
          response: {
            paused: false,
            message: "approved 1 pending deletion(s); run `proton-sync syncnow` to apply now",
          },
        };
      case "keep":
        return {
          state: "idle",
          error: null,
          response: {
            paused: false,
            message: "kept 1 pending deletion(s); the other side is put back on the next sync",
          },
        };
      case "run_dry_run":
        return this.planFor(args?.pair);
      case "apply_plan":
        return { state: "applied", apply_seq: 1, executed: 1, skipped_destructive: 0, failed: 0 };
      case "sync_now":
        return this.status();
      default:
        return null;
    }
  }
}

const { server, port } = await serve();
const browser = await puppeteer.launch({ headless: true, args: ["--no-sandbox"] });
const failures = [];

/** A fresh page wired to a fresh bridge. */
async function open(bridge) {
  const page = await browser.newPage();
  await page.setViewport({ width: 1040, height: 764, deviceScaleFactor: 1 });
  await page.exposeFunction("__bridge", (cmd, args) => bridge.handle(cmd, args));
  await page.evaluateOnNewDocument(() => {
    window.__listeners = {};
    // The injection Tauri makes, as an object literal: the lint rule that keeps the app's own code
    // on the `api` facade matches the dotted `window.__TAURI__` and this is the one place the
    // injection is MADE rather than reached for.
    Object.assign(window, {
      __TAURI__: {
        core: { invoke: (cmd, args) => window.__bridge(cmd, args ?? null) },
        event: {
          listen: (name, callback) => {
            window.__listeners[name] = callback;
            return Promise.resolve(() => {});
          },
        },
      },
    });
  });
  page.on("pageerror", (error) => failures.push(`page error: ${error.message}`));
  await page.goto(`http://127.0.0.1:${port}/index.html`, { waitUntil: "networkidle0" });
  return page;
}

/** Click the first button whose text is exactly `text`; waits for it to exist. */
async function press(page, text) {
  await until(`a button reading "${text}"`, () =>
    page.evaluate(
      (label) => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === label),
      text,
    ),
  );
  await page.evaluate((label) => {
    [...document.querySelectorAll("button")].find((b) => b.textContent.trim() === label).click();
  }, text);
}

/** The selection moves: the bridge answers for the other pair, and the `pair-selected` event fires. */
async function select(page, bridge, pair) {
  bridge.selected = pair;
  const before = bridge.called("get_status").length;
  await page.evaluate((name) => window.__listeners["pair-selected"]({ payload: name }), pair);
  await until("a status poll after the switch", () => bridge.called("get_status").length > before);
  await delay(150); // the reply lands, the store moves, the screen re-renders
}

async function scenario(name, run) {
  const before = failures.length;
  try {
    await run();
  } catch (error) {
    failures.push(`${name}: ${error.message}`);
  }
  if (failures.length === before) console.log(`fidelity:pairs — ${name}: ok`);
}

function expectPair(calls, pair, what) {
  if (calls.length === 0) throw new Error(`${what}: nothing was sent`);
  const wrong = calls.filter((call) => call.args?.pair !== pair);
  if (wrong.length) {
    throw new Error(
      `${what}: sent for ${JSON.stringify(wrong.map((c) => c.args?.pair))}, expected ${JSON.stringify(pair)} ` +
        `(the selection was ${JSON.stringify(wrong.map((c) => c.selectedThen))} when it arrived)`,
    );
  }
}

// ---- 1. the tail of a decision ---------------------------------------------------------------------
await scenario("an approval's follow-up goes to the pair the approval was for", async () => {
  const bridge = new Bridge({ docs: [deletion("a.txt")], photos: [] });
  const page = await open(bridge);
  await press(page, MAIN.band.deletionAction); // the band's `Review`
  const release = bridge.hold("approve");
  await press(page, DELETIONS.toTrash);
  await until("the approval", () => bridge.called("approve").length === 1);
  expectPair(bridge.called("approve"), "docs", "approve");

  await select(page, bridge, "photos"); // while the approval is still in flight
  release();
  await until("the sync nudge", () => bridge.called("sync_now").length === 1);
  expectPair(bridge.called("sync_now"), "docs", "sync_now after the switch");
  await page.close();
});

// ---- 2. the press of `Run this sync` ---------------------------------------------------------------
await scenario(
  "a press of Run this sync applies the plan it reviewed, to the pair it was made for",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] });
    const page = await open(bridge);
    await press(page, "Plan a sync");
    await until("the rehearsal", () => bridge.called("run_dry_run").length === 1);
    expectPair(bridge.called("run_dry_run"), "docs", "run_dry_run");
    await until("the gate", () => page.$(".delete-gate"));
    await page.type(".delete-gate", "DELETE");

    const release = bridge.hold("approve");
    await press(page, PLAN.run);
    await until("the first approval", () => bridge.called("approve").length === 1);
    expectPair(bridge.called("approve"), "docs", "the plan's deletion approval");

    await select(page, bridge, "photos"); // resets the plan screen, which is rehearsing for photos now
    release();
    await until("the apply", () => bridge.called("apply_plan").length === 1);
    expectPair(bridge.called("apply_plan"), "docs", "apply_plan after the switch");
    const applied = bridge.called("apply_plan")[0].args;
    if (applied.token !== "token-for-docs") {
      throw new Error(`apply_plan carried ${JSON.stringify(applied.token)}, not the plan that was reviewed`);
    }
    await page.close();
  },
);

// ---- 3. a switch rebuilds the cards ------------------------------------------------------------------
await scenario("after a switch, Keep on the card now showing goes to the pair now showing", async () => {
  // The SAME path at the SAME fingerprint in both folders: the two queues draw identical cards.
  const same = deletion("same.txt");
  const bridge = new Bridge({ docs: [same], photos: [same] });
  const page = await open(bridge);
  await press(page, MAIN.band.deletionAction);
  await until("the card", () =>
    page.evaluate(
      (label) => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === label),
      DELETIONS.keepLocal,
    ),
  );
  await select(page, bridge, "photos");
  await press(page, DELETIONS.keepLocal);
  await until("the keep", () => bridge.called("keep").length === 1);
  expectPair(bridge.called("keep"), "photos", "keep");
  await page.close();
});

// ---- 4. the hero's buttons are the hero's -----------------------------------------------------------
await scenario("after a switch, the hero's Pause pauses the pair the hero is now about", async () => {
  // Same hero for both pairs (settled), so the screen is PATCHED across the switch and its buttons are
  // not rebuilt. A button bound when it was built would go on acting on the pair it was built for.
  const bridge = new Bridge({ docs: [], photos: [] });
  const page = await open(bridge);
  await until("the settled hero", () =>
    page.evaluate(
      (label) => [...document.querySelectorAll("button")].some((b) => b.textContent.trim() === label),
      MAIN.pause,
    ),
  );
  await select(page, bridge, "photos");
  await press(page, MAIN.pause);
  await until("the pause", () => bridge.called("pause").length === 1);
  expectPair(bridge.called("pause"), "photos", "pause");
  await page.close();
});

await browser.close();
server.close();

if (failures.length) {
  console.error(`\nfidelity:pairs — ${failures.length} failure(s):\n`);
  for (const failure of failures) console.error(`  ${failure}\n`);
  console.error("A write must act on the pair it was drawn for, not on whichever is selected when it runs.");
  process.exit(1);
}
console.log("fidelity:pairs — every write acted on the pair it was drawn for");
