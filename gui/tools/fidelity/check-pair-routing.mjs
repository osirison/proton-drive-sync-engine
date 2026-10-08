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
// Ten scenarios. The first four are each a way a write can land on a different pair than the one it was
// drawn for — each was a bug before the capture existed or would be again if it were removed. The fifth
// and sixth are the first-run rule at two pairs and at one. The next three are the rest of the capture:
// a late READ, the decision on a conflict, and the tray panel's pin. The last is the tray panel's rows
// at two folders (#102 phase 5d).
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
//   5. A PAIR THAT HAS NEVER SYNCED. At two folders the first-run wizard (whose `Next` writes
//      top-level roots a `[[pair]]` file refuses) must stay shut, and the window must say the pair has
//      not synced rather than `Everything is up to date`; ...
//   6. ... and at one folder the same reply still opens the wizard, because it is the count and not
//      the state that decides.
//   7. A LATE READ. Two folders hold a conflict at the SAME relative path, so the reads for them differ
//      only in the folder they were issued for. The older read answers after the selection has moved
//      and the newer one has been issued; its content must not become the card now showing, on the
//      screen where the person chooses which version to destroy.
//   8. THE DECISION ON A CONFLICT. A card drawn for one folder is pressed after the selection has
//      moved to another; the decision goes to the folder the card was drawn for.
//   8b. ... AND WHAT FOLLOWS IT. The decision lands after the selection has moved: its continuation
//      (the tally, the position, the cached bytes) belongs to the screen that was shown, not the one
//      that is, and must not touch the folder now on screen.
//   9. THE TRAY PANEL'S PIN. The panel shows the default pair whatever the window has selected, from its
//      very FIRST poll: it asks for it with a command of its own (`tray_status`) that has no pair to
//      name, so there is no first call that can mean the window's selection (phase 5a-2 left that as a
//      recorded gap: before the roster was heard the poll named no pair, which Rust reads as the
//      selected one).
//  10. THE TRAY PANEL'S ROWS AT TWO FOLDERS. The panel is the worst folder's, named, with one pause row
//      per folder; pressing a folder's row sends THAT folder's id, and the panel is repainted from the
//      status that comes back, not from the folder the row was for.
//
// WHAT IT CANNOT SEE: it scripts the bridge, so it proves the facade and the screens agree with each
// other, not that the real Rust agrees with either (that is `selection_tests.rs`); and it drives the
// window, not the tray panel.

import puppeteer from "puppeteer";
import { serve } from "./serve.mjs";
import { CONFLICTS, DELETIONS, MAIN, PLAN, TRAY } from "../../src/js/ui/copy.js";
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

/** One conflict, found at the same relative path in both folders: the reads for them differ in the folder alone. */
const CONFLICT = { original: "note.txt", sidecar: "note.proton-cloud.txt", kind: "content" };

/** A scripted daemon: answers every command the window sends, records them, and holds replies on request. */
class Bridge {
  constructor(
    queues,
    { names = PAIRS, neverSynced = [], conflicts = {}, selected = "docs", states = {} } = {},
  ) {
    this.names = names;
    this.neverSynced = neverSynced;
    // pair -> `{ state, rank, summary }`: what Rust derived for a folder that is not simply idle.
    this.states = states;
    this.conflicts = conflicts; // pair -> the conflicts a scan finds there
    this.calls = [];
    this.selected = selected;
    this.queues = queues; // pair -> pending deletions
    this.holds = new Map(); // command (or `command:pair`) -> { release }
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

  /**
   * The reply to a status read. `name` is the pair it DESCRIBES — the one the request named, else the
   * selected one, as Rust answers — and `selected` is the app's choice, which is not the same thing.
   */
  status(name = this.selected) {
    const never = (pair) => this.neverSynced.includes(pair);
    const pairs = this.names.map((pair) => ({
      ...summaryOf(pair, this.queues[pair].length),
      ...(never(pair) ? { last_sync_epoch_secs: null } : {}),
      ...(this.states[pair]?.summary ?? {}),
    }));
    return {
      // What `derive_state` says: a reachable daemon that has never synced THIS pair.
      state: never(name) ? "firstRun" : "idle",
      selected: this.selected,
      pairs,
      pair_states: this.names.map((pair) => ({
        name: pair,
        state: this.states[pair]?.state ?? "idle",
        rank: this.states[pair]?.rank ?? 0,
      })),
      response: {
        status: "running",
        paused: false,
        syncing: false,
        reconcile_seq: 1,
        pending_changes: 0,
        message: "",
        last_sync_epoch_secs: never(name) ? null : 1_750_000_000,
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

  /**
   * Hold the next reply to `command` until `release()` is called — to `command` sent for `pair` when
   * one is given, so two reads of the same command for two folders can be released in either order.
   */
  hold(command, pair) {
    let release;
    const released = new Promise((resolve) => {
      release = resolve;
    });
    this.holds.set(pair ? `${command}:${pair}` : command, { released, release });
    return release;
  }

  called(command) {
    return this.calls.filter((call) => call.cmd === command);
  }

  async handle(cmd, args) {
    this.calls.push({ cmd, args, selectedThen: this.selected });
    for (const key of [`${cmd}:${args?.pair}`, cmd]) {
      const held = this.holds.get(key);
      if (held) {
        this.holds.delete(key);
        await held.released;
        break;
      }
    }
    switch (cmd) {
      case "fence":
        return null;
      case "get_status":
        return this.status(args?.pair ?? this.selected);
      case "tray_status":
        // What Rust answers: the DEFAULT pair, whatever the window has selected and whatever is asked.
        return this.status(this.names[0]);
      case "tray_action":
        // A row of the panel: the reply is the panel's own status, never the addressed folder's.
        return this.status(this.names[0]);
      case "read_config":
        return {
          ...EMPTY_CONFIG,
          exists: true,
          pairs: this.names.map((name) => ({
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
        return this.conflicts[args?.pair ?? this.selected] ?? [];
      case "read_conflict_pair": {
        // Content that names its folder, so a card that shows the wrong folder's file says so in text.
        const side = (kind) => ({
          exists: true,
          size: 40,
          mtime_epoch_secs: 1_750_000_000,
          text: `MARK-${args?.pair}-${kind}\nline two`,
          binary_or_large: false,
        });
        return { original: side("mine"), sidecar: side("theirs"), happened: null };
      }
      case "resolve_conflict":
        return null;
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

/** A fresh page wired to a fresh bridge. `query` is the page's own (`?surface=tray` opens the tray panel). */
async function open(bridge, query = "") {
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
  await page.goto(`http://127.0.0.1:${port}/index.html${query}`, { waitUntil: "networkidle0" });
  return page;
}

/**
 * Wait until the page has PROCESSED every reply the bridge has already sent it. A call made now is
 * answered after them, and replies reach the page in the order they were sent, so by the time this
 * one's answer is back each earlier reply's continuation has run — the store has moved and the
 * screen has been drawn. It is a condition, where a sleep was a guess at how long that takes.
 */
const settle = (page) => page.evaluate(() => window.__bridge("fence", null));

const pageText = (page) => page.evaluate(() => document.getElementById("app-root")?.innerText ?? "");

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
  await settle(page); // the reply lands, the store moves, the screen re-renders
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

// ---- 5. a pair that has never synced --------------------------------------------------------------
await scenario(
  "at two pairs a never-synced pair is drawn as such, and the first-run takeover stays shut",
  async () => {
    const bridge = new Bridge({ docs: [], photos: [] }, { neverSynced: ["docs"] });
    const page = await open(bridge);
    await until("the hero", async () => (await pageText(page)).includes(TRAY.nothingSyncedYet));
    const shown = await pageText(page);
    if (shown.includes(MAIN.settled))
      throw new Error(`a pair that has never synced was drawn as "${MAIN.settled}"`);
    if (!shown.includes(TRAY.nothingSyncedYetSub))
      throw new Error("the second sentence of the never-synced hero is missing");
    // The takeover is the full-window wizard, which draws its own chip and no ⋯ menu.
    if (/step 1 of 2/.test(shown)) throw new Error("the first-run takeover opened over a running app");
    await page.close();
  },
);

await scenario("at one pair the same reply still opens the first-run takeover", async () => {
  // The other half of the rule: it is the COUNT that keeps the wizard shut, not the state.
  const bridge = new Bridge({ docs: [] }, { names: ["docs"], neverSynced: ["docs"] });
  const page = await open(bridge);
  await until("the takeover", async () => /step 1 of 2/.test(await pageText(page)));
  await page.close();
});

// ---- 7. a late read ---------------------------------------------------------------------------------
await scenario(
  "a late conflict read for the pair that was left never fills the card now showing",
  async () => {
    const bridge = new Bridge(
      { docs: [], photos: [] },
      { conflicts: { docs: [CONFLICT], photos: [CONFLICT] } },
    );
    const releaseDocs = bridge.hold("read_conflict_pair", "docs");
    const releasePhotos = bridge.hold("read_conflict_pair", "photos");
    const page = await open(bridge);
    await press(page, MAIN.band.conflictAction);
    await until("docs' read", () => bridge.called("read_conflict_pair").some((c) => c.args?.pair === "docs"));

    await select(page, bridge, "photos"); // while docs' read is still out
    await until("photos' read", () =>
      bridge.called("read_conflict_pair").some((c) => c.args?.pair === "photos"),
    );

    releaseDocs(); // the OLDER read answers first, after the selection has moved on
    await settle(page);
    releasePhotos();
    await settle(page);
    await press(page, CONFLICTS.showDiff);
    const shown = await pageText(page);
    if (/MARK-docs/.test(shown)) {
      throw new Error("the card for photos shows docs' file content, read for a different folder");
    }
    // The positive control: the card is not merely blank, it has photos' own content.
    if (!/MARK-photos-mine/.test(shown))
      throw new Error("the card for photos does not show photos' own content");
    await page.close();
  },
);

// ---- 8. the decision on a conflict ----------------------------------------------------------------------
await scenario("a decision on a conflict goes to the pair the card was drawn for", async () => {
  const bridge = new Bridge(
    { docs: [], photos: [] },
    { conflicts: { docs: [CONFLICT], photos: [CONFLICT] } },
  );
  const page = await open(bridge);
  await press(page, MAIN.band.conflictAction);
  const keepBoth = await until("the card's choices", async () => {
    const handle = await page.evaluateHandle(
      (label) =>
        [...document.querySelectorAll("button")].find(
          (b) => b.querySelector(".btn-choice-name")?.textContent.trim() === label,
        ) ?? null,
      CONFLICTS.keepBoth,
    );
    return handle.asElement();
  });
  // The selection moves under a card that has not been redrawn yet, and the press lands on THAT card:
  // the button is held across the switch, which is what a click arriving in the gap looks like.
  await select(page, bridge, "photos");
  await keepBoth.evaluate((button) => button.click());
  await until("the decision", () => bridge.called("resolve_conflict").length === 1);
  expectPair(bridge.called("resolve_conflict"), "docs", "resolve_conflict");
  await page.close();
});

await scenario(
  "a late decision on one pair's conflict leaves the card another pair is showing where it was",
  async () => {
    // docs has one conflict and photos two. The decision on docs' is out when the selection moves to
    // photos, and photos' queue is stepped to its SECOND card. The decision then lands: it must settle
    // docs' conflict and touch nothing the photos screen holds. Unguarded, its continuation set the
    // position from docs' rescan (one conflict, so index 0) and the photos screen jumped back to its first.
    const SECOND = { original: "other.txt", sidecar: "other.proton-cloud.txt", kind: "content" };
    const bridge = new Bridge(
      { docs: [], photos: [] },
      { conflicts: { docs: [CONFLICT], photos: [CONFLICT, SECOND] } },
    );
    const releaseDecision = bridge.hold("resolve_conflict");
    const page = await open(bridge);
    await press(page, MAIN.band.conflictAction);
    const keepBoth = await until("the card's choices", async () => {
      const handle = await page.evaluateHandle(
        (label) =>
          [...document.querySelectorAll("button")].find(
            (b) => b.querySelector(".btn-choice-name")?.textContent.trim() === label,
          ) ?? null,
        CONFLICTS.keepBoth,
      );
      return handle.asElement();
    });
    await keepBoth.evaluate((button) => button.click());
    await until("the decision to go out", () => bridge.called("resolve_conflict").length === 1);
    await select(page, bridge, "photos"); // while the decision is still out
    await press(page, "\u203a"); // `›`: photos' second card
    await until("photos' second card", async () => /other\.txt/.test(await pageText(page)));

    releaseDecision();
    await settle(page);
    await until("the rescan of docs", () =>
      bridge.called("scan_conflicts").some((c) => c.args?.pair === "docs"),
    );
    await settle(page);
    const shown = await pageText(page);
    if (!/other\.txt/.test(shown)) {
      throw new Error("the late decision for docs moved the photos screen off its second card");
    }
    expectPair(bridge.called("resolve_conflict"), "docs", "resolve_conflict");
    await page.close();
  },
);

// ---- 9. the tray panel's pin ------------------------------------------------------------------------------
await scenario("the tray panel shows the pair its rows act on, not the one the window selected", async () => {
  // The window has selected `photos`. The panel's rows act on the default pair (`docs`), which has never
  // synced here: a panel that followed the selection would say `Everything is up to date` beside rows
  // that act on a folder it is not describing.
  const bridge = new Bridge({ docs: [], photos: [] }, { selected: "photos", neverSynced: ["docs"] });
  const page = await open(bridge, "?surface=tray");
  await until("the first poll", () => bridge.called("tray_status").length >= 1);
  await settle(page);
  const before = bridge.called("tray_status").length;
  await page.evaluate(() => window.__listeners["pair-selected"]({ payload: "photos" }));
  await until("a poll after the selection moved", () => bridge.called("tray_status").length > before);
  await settle(page);

  // THE FIRST POLL TOO. It is a command with no pair to name, so it cannot mean the selection; and the
  // panel never asks `get_status`, whose unnamed read IS the selection.
  const asked = bridge.calls.filter((call) => call.cmd === "tray_status" || call.cmd === "get_status");
  const wrong = asked.filter((call) => call.cmd !== "tray_status" || call.args != null);
  if (wrong.length) {
    throw new Error(
      `the panel asked ${JSON.stringify(wrong.map((c) => [c.cmd, c.args]))} instead of a bare tray_status`,
    );
  }
  const shown = await pageText(page);
  if (!shown.includes(TRAY.nothingSyncedYet)) {
    throw new Error(
      `the panel does not describe the default pair (docs, never synced): ${JSON.stringify(shown)}`,
    );
  }
  if (shown.includes(MAIN.compact.upToDate))
    throw new Error("the panel drew the selected pair (photos) as up to date");
  await page.close();
});

// ---- 10. the tray panel's rows at two folders --------------------------------------------------------------
await scenario(
  "the tray panel at two folders names the worst one and a folder row goes to its folder",
  async () => {
    // `photos` failed; `docs` (the default, the folder the reply describes) is fine.
    const bridge = new Bridge(
      { docs: [], photos: [] },
      {
        selected: "docs",
        states: { photos: { state: "failed", rank: 4, summary: { last_error: "boom", pending_changes: 3 } } },
      },
    );
    const page = await open(bridge, "?surface=tray");
    await until("the first poll", () => bridge.called("tray_status").length >= 1);
    await settle(page);

    const shown = await pageText(page);
    if (!shown.includes("photos"))
      throw new Error(`the panel does not name the worst folder: ${JSON.stringify(shown)}`);
    if (!shown.includes(MAIN.failed))
      throw new Error(`the panel is not the failed folder's: ${JSON.stringify(shown)}`);
    if (shown.includes(MAIN.compact.upToDate))
      throw new Error("the panel drew an up-to-date folder over a failed one");
    if (shown.includes("boom")) throw new Error("the daemon's own sentence reached a 362px panel");
    if (shown.includes(TRAY.pause) || shown.includes(TRAY.resume)) {
      throw new Error(`a row says ${TRAY.pause}/${TRAY.resume} at two folders: ${JSON.stringify(shown)}`);
    }

    await press(page, TRAY.pausePair("photos"));
    await until("the row to go out", () => bridge.called("tray_action").length === 1);
    const sent = bridge.called("tray_action")[0].args;
    if (sent?.id !== "pause@photos") {
      throw new Error(
        `pressing "${TRAY.pausePair("photos")}" sent ${JSON.stringify(sent)}, not pause@photos`,
      );
    }
    // And the reply the panel is handed is the default folder's own status, so the panel still shows what
    // it showed — the failed folder — rather than repainting as the folder the row was for.
    await settle(page);
    const after = await pageText(page);
    if (!after.includes("photos") || !after.includes(MAIN.failed)) {
      throw new Error(`the panel changed after a folder's row: ${JSON.stringify(after)}`);
    }
    await page.close();
  },
);

await browser.close();
server.close();

if (failures.length) {
  console.error(`\nfidelity:pairs — ${failures.length} failure(s):\n`);
  for (const failure of failures) console.error(`  ${failure}\n`);
  console.error("A write must act on the pair it was drawn for, not on whichever is selected when it runs.");
  process.exit(1);
}
console.log("fidelity:pairs — every write acted on the pair it was drawn for");
