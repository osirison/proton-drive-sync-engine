// S6's model. Like S5's, most of it is code no frame draws.
//
// Three of the four tabs are drawn and the fourth is not; the skip tab's rule row has five possible
// second lines and the frames give two; the deletion policy has four combinations and three cards.
// So the fidelity gate — which compares the app against the frames — cannot see the states these
// assertions are about, and the copy gate never renders the app at all.
//
// TWO OF THESE ARE SAFETY PROPERTIES rather than correctness ones, and they are the reason this file
// leads with the rule rows: `no such folder here any more — safe to remove` is a sentence someone
// acts on without checking, and `Saving writes only what you changed` is a promise about a file
// nobody asked us to rewrite. Both are asserted at the cell the frames do not draw.

import { test } from "node:test";
import assert from "node:assert/strict";

import {
  POLICIES,
  policyOf,
  ruleEffect,
  totalLine,
  removalCost,
  configUpdate,
  isDirty,
  refusalReason,
  intervalLabel,
  stepInterval,
  barActionOf,
  barNoteOf,
  settingsBarShape,
  MIN_INTERVAL_SECS,
  MAX_INTERVAL_SECS,
} from "../src/js/screens/settings.js";
import { SETTINGS } from "../src/js/ui/copy.js";

// ---- the rule row's five second lines --------------------------------------------------------

const rule = (over = {}) => ({
  pattern: "*.tmp",
  files: 0,
  bytes: 0,
  unique_files: 0,
  unique_bytes: 0,
  samples: [],
  folder_exists: null,
  error: null,
  ...over,
});

test("a bare glob names the files it hides — the drawn `*.tmp` row", () => {
  const effect = ruleEffect(
    rule({
      files: 2,
      bytes: 2_940_000,
      samples: [{ path: "exports/draft.tmp" }, { path: "exports/render-final.tmp" }],
    }),
  );
  assert.equal(effect.effect, SETTINGS.skippingNow(2));
  assert.equal(effect.detail, "exports/draft.tmp, exports/render-final.tmp");
  assert.equal(effect.dim, false);
});

test("a folder rule describes itself by size — the drawn `video-raw/**` row", () => {
  const effect = ruleEffect(
    rule({
      pattern: "video-raw/**",
      files: 2,
      bytes: 3_100_000_000,
      samples: [{ path: "video-raw/a-roll.mov" }, { path: "video-raw/b-roll.mov" }],
      folder_exists: true,
    }),
  );
  assert.equal(effect.effect, SETTINGS.skippingSize(2, 3_100_000_000));
  assert.equal(effect.detail, SETTINGS.ruleFolderHere);
});

test("a bare glob with more matches than samples does NOT name four of fifty", () => {
  // `MAX_SAMPLES` is 4. A list of four under `Skipping 50 files right now` reads as the whole set,
  // which is the one thing the sub-line must not do.
  const effect = ruleEffect(
    rule({ files: 50, bytes: 900, samples: [{ path: "a" }, { path: "b" }, { path: "c" }, { path: "d" }] }),
  );
  assert.equal(effect.effect, SETTINGS.skippingSize(50, 900));
  assert.equal(effect.detail, null);
});

test("matching nothing with the folder gone is the one dimmed, removable row", () => {
  const effect = ruleEffect(rule({ pattern: "old-backups/**", folder_exists: false }));
  assert.equal(effect.effect, SETTINGS.matchingNothing);
  assert.equal(effect.detail, SETTINGS.staleRule);
  assert.equal(effect.dim, true);
});

test("matching nothing with the folder still there is idle, not removable", () => {
  // An empty folder that still exists matches nothing and is NOT safe to remove — §69b's split,
  // and a cell no frame draws.
  const effect = ruleEffect(rule({ pattern: "video-raw/**", folder_exists: true }));
  assert.equal(effect.effect, SETTINGS.matchingNothing);
  assert.equal(effect.detail, SETTINGS.ruleFolderHere);
  assert.equal(effect.dim, false, "an idle rule must not be dimmed as though it were removable");
});

test("a rule that IS hiding files is never called safe to remove", () => {
  // The destructive direction. `folder_exists:false` with matches is odd data — a pattern whose
  // literal prefix is gone but which still matches elsewhere — and the answer that costs someone
  // their files is `safe to remove`, so that is the one that must be unreachable.
  for (const folder_exists of [false, true, null]) {
    const effect = ruleEffect(rule({ files: 3, bytes: 10, folder_exists }));
    assert.notEqual(effect.detail, SETTINGS.staleRule);
    assert.equal(effect.dim, false);
  }
});

test("a rule the walk has not measured is not a rule that matches nothing", () => {
  // The tab fires the local-tree walk unawaited on every visit, so for the length of that walk —
  // seconds on a large folder — the report says nothing about any rule. `files ?? 0` turned that
  // into `Matching nothing`, which is "unknown is never zero" broken on the one line that invites
  // someone to delete a rule that may be hiding forty gigabytes.
  const unmeasured = ruleEffect({ pattern: "video-raw/**" });
  assert.equal(unmeasured.effect, SETTINGS.ruleChecking);
  assert.equal(unmeasured.detail, null);
  assert.notEqual(unmeasured.effect, SETTINGS.matchingNothing);
  // A measured zero still says so.
  assert.equal(ruleEffect(rule({ files: 0 })).effect, SETTINGS.matchingNothing);
});

test("a rule the walk could not evaluate says so, in the daemon's own words", () => {
  const effect = ruleEffect(rule({ error: "invalid glob: unterminated [" }));
  assert.equal(effect.effect, SETTINGS.ruleUnchecked);
  assert.equal(effect.detail, "invalid glob: unterminated [");
  // NOT `Matching nothing`: nothing was measured, which is not the same as nothing matched.
  assert.notEqual(effect.effect, SETTINGS.matchingNothing);
});

// ---- the total, and the floor it becomes -----------------------------------------------------

test("the total is the report's distinct-file union", () => {
  const line = totalLine({ total_files: 4, total_bytes: 3_102_940_000, unreadable_directories: 0 });
  assert.equal(line, SETTINGS.hidingTotal(4, 3_102_940_000));
});

test("an unreadable directory turns every number on the tab into a floor", () => {
  for (const over of [{ unreadable_directories: 1 }, { unreadable_entries: 2 }]) {
    const line = totalLine({ total_files: 4, total_bytes: 100, ...over });
    assert.equal(line, SETTINGS.hidingFloor(4, 100));
    assert.notEqual(line, SETTINGS.hidingTotal(4, 100));
  }
});

test("no report means no sentence, not a sentence about zero", () => {
  assert.equal(totalLine(null), null);
});

// ---- the removal cost line -------------------------------------------------------------------

const report = {
  rules: [
    { pattern: "*.tmp", files: 2, bytes: 9, unique_files: 2, unique_bytes: 9 },
    { pattern: "video-raw/**", files: 5, bytes: 99, unique_files: 2, unique_bytes: 3_100_000_000 },
  ],
};

test("the cost line counts the files ONLY that rule hides", () => {
  const line = removalCost(["*.tmp", "video-raw/**"], ["*.tmp"], report);
  // `unique_*`, not `files`/`bytes`: a file a second rule also hides does not start syncing.
  assert.equal(line, SETTINGS.ruleRemovedCost(2, 3_100_000_000));
});

test("two rules removed leaves the neutral note — the deck has no plural for it", () => {
  assert.equal(removalCost(["*.tmp", "video-raw/**"], [], report), null);
});

test("removing a rule that hides nothing is not a cost — the amber line stays away", () => {
  // The commonest safe edit on this tab: `safe to remove` invites it, and
  // `One rule removed — 0 files, 0 B will start syncing.` would be both true and a false alarm.
  const stale = { rules: [{ pattern: "old-backups/**", unique_files: 0, unique_bytes: 0 }] };
  assert.equal(removalCost(["old-backups/**"], [], stale), null);
});

test("an addition is not a cost, and neither is an unmeasured rule", () => {
  assert.equal(removalCost(["*.tmp"], ["*.tmp", "*.psd"], report), null);
  assert.equal(removalCost(["unknown/**"], [], report), null);
});

// ---- what a save actually writes -------------------------------------------------------------

const config = {
  local_root: "~/ProtonDrive",
  remote_root: "/Drive/RemoteFolder",
  scan_interval_secs: 300,
  events_driven: true,
  exclude: ["*.tmp"],
  proton_timeout_secs: 60,
};

test("a save writes the changed field and nothing else", () => {
  // `Saving writes only what you changed` is a wire contract: `write_config` edits the TOML in
  // place, so a field sent at all is a key written into a file that may never have had the line.
  const update = configUpdate(config, { events_driven: false, scan_interval_secs: 300 });
  assert.deepEqual(update, { events_driven: false });
});

test("an array compares by content, not by identity", () => {
  // `exclude` is staged as a new array on every edit, so removing and re-adding the same rule must
  // come back to "nothing changed".
  assert.deepEqual(configUpdate(config, { exclude: ["*.tmp"] }), {});
  assert.deepEqual(configUpdate(config, { exclude: ["*.tmp", "*.psd"] }), {
    exclude: ["*.tmp", "*.psd"],
  });
  assert.deepEqual(configUpdate(config, { exclude: [] }), { exclude: [] });
});

test("an absent key equals its default, so re-picking what is shown writes nothing", () => {
  // `read_config` returns null for a key the file does not have and the screen draws the daemon's
  // default in its place. Clicking the card that is already selected must not materialise two keys
  // the file never had — the footer promises the opposite.
  const fresh = { local_root: "~/ProtonDrive" };
  assert.deepEqual(
    configUpdate(fresh, {
      delete_approval_remote: true,
      delete_approval_local: true,
      deletion_policy: "ask_every_time",
    }),
    {},
  );
  assert.deepEqual(configUpdate(fresh, { events_driven: true }), {});
  // The timer draws `5 min` on a silent config, so stepping up and back down to it writes nothing.
  assert.deepEqual(configUpdate(fresh, { scan_interval_secs: 300 }), {});
  assert.deepEqual(configUpdate(fresh, { scan_interval_secs: 360 }), { scan_interval_secs: 360 });
  // And a real change from the default is still a change.
  assert.deepEqual(configUpdate(fresh, { events_driven: false }), { events_driven: false });
  assert.deepEqual(configUpdate(fresh, { delete_approval_local: false }), { delete_approval_local: false });
  // A key with no default is drawn empty when absent, so setting it IS a change.
  assert.deepEqual(configUpdate(fresh, { remote_root: "/Drive/x" }), { remote_root: "/Drive/x" });
});

test("dirty is exactly `there is something to write`", () => {
  assert.equal(isDirty(config, {}), false);
  assert.equal(isDirty(config, { events_driven: true }), false, "staging the current value is not a change");
  assert.equal(isDirty(config, { events_driven: false }), true);
});

// ---- the deletion policy, and its fourth state -----------------------------------------------

test("the three cards are the three drawn pairs, in the drawn order", () => {
  assert.deepEqual(
    POLICIES.map((p) => p.id),
    ["ask_every_time", "only_permanent", "never"],
  );
  assert.equal(policyOf({ delete_approval_remote: true, delete_approval_local: true }), "ask_every_time");
  assert.equal(policyOf({ delete_approval_remote: false, delete_approval_local: true }), "only_permanent");
  assert.equal(policyOf({ delete_approval_remote: false, delete_approval_local: false }), "never");
});

test("a config written with the deletion_policy key selects its card, not the default", () => {
  // #194. A file spelled `deletion_policy = "never"` reports BOTH booleans as absent, and absent
  // means `true` — so reading the card off the booleans drew `Ask me every time` over a machine
  // that asks about nothing. `read_config` resolves whichever spelling the file uses and this is
  // what reads its answer.
  assert.equal(policyOf({ deletion_policy: "never" }), "never");
  assert.equal(policyOf({ deletion_policy: "only_permanent" }), "only_permanent");
  assert.equal(
    policyOf({ deletion_policy: "never", delete_approval_remote: true, delete_approval_local: true }),
    "never",
    "the resolved policy outranks booleans that a policy-spelled file never set",
  );
  assert.equal(policyOf({ deletion_policy: "only_recoverable" }), null, "the undrawn one draws no card");
  assert.equal(policyOf({ deletion_policy: "something_else" }), null, "an unknown value draws no card");
});

test("the fourth combination selects NO card rather than the nearest one", () => {
  // `remote:true, local:false` — ask before Proton's Trash, wipe local files for good without
  // asking — is reachable by hand-editing the config. Coercing it to a card would mean the next
  // save silently rewrote a live safety policy nobody touched. DEVIATIONS §68.
  assert.equal(policyOf({ delete_approval_remote: true, delete_approval_local: false }), null);
});

test("an absent key means asking, not `Never ask`", () => {
  assert.equal(policyOf({}), "ask_every_time");
  assert.equal(policyOf(null), null);
});

test("every card writes both booleans", () => {
  for (const policy of POLICIES) {
    assert.equal(typeof policy.remote, "boolean");
    assert.equal(typeof policy.local, "boolean");
  }
});

// ---- the daemon's refusal --------------------------------------------------------------------

test("the mono box shows the reason, not the sentence wrapped around it", () => {
  assert.equal(
    refusalReason("config would be rejected by the daemon: remote_root: /Drive/Archive2026 — not found"),
    "remote_root: /Drive/Archive2026 — not found",
  );
});

test("any other message is quoted whole — voice rule 4", () => {
  assert.equal(
    refusalReason("permission denied: /etc/proton-sync.toml"),
    "permission denied: /etc/proton-sync.toml",
  );
  assert.equal(refusalReason(""), null);
  assert.equal(refusalReason(null), null);
});

// ---- the interval, in plain language ----------------------------------------------------------

test("whole minutes read as minutes and anything else reads in seconds", () => {
  assert.equal(intervalLabel(300), SETTINGS.timerUnit(5));
  assert.equal(intervalLabel(60), SETTINGS.timerUnit(1));
  // A config written by hand at 90s must not draw as `2 min`: the stepper would then write 120 for
  // a value nobody touched.
  assert.equal(intervalLabel(90), SETTINGS.timerSeconds(90));
});

test("the stepper moves a minute at a time and clamps at both ends", () => {
  assert.equal(stepInterval(300, 1), 360);
  assert.equal(stepInterval(300, -1), 240);
  assert.equal(stepInterval(MIN_INTERVAL_SECS, -1), MIN_INTERVAL_SECS);
  assert.equal(stepInterval(MAX_INTERVAL_SECS, 1), MAX_INTERVAL_SECS);
  // An odd interval keeps its remainder rather than being rounded on the way past.
  assert.equal(stepInterval(90, 1), 150);
});

// ---- the footer bar ----------------------------------------------------------------------------

test("the bar says what just happened first, and only then the standing information", () => {
  // A control that failed outranks a cost line: the cost is about a change not yet made, and
  // reporting it over the failure would put the silence back one layer up.
  assert.equal(barNoteOf({ notice: "x", cost: "c", note: "n" }), "x");
  assert.equal(barNoteOf({ cost: "c", note: "n" }), "c");
  assert.equal(barNoteOf({ note: "n" }), "n");
  assert.equal(barNoteOf({}), SETTINGS.saveNote);
});

test("a save that would interrupt a running sync says so before the click", () => {
  // #320. Saving restarts the sync service now, so the bar has to warn while there is something
  // staged AND a pass in flight — the decision accepts a brief interruption and refuses one nobody
  // saw coming.
  assert.equal(barNoteOf({ dirty: true, syncing: true }), SETTINGS.saveInterrupts);
  // NOT HIDDEN BY THE COST LINE, which is the one ordering this decides: the cost describes what a
  // staged change lets through once saved, this describes what the click does to a transfer that is
  // happening now.
  assert.equal(barNoteOf({ dirty: true, syncing: true, cost: "c" }), SETTINGS.saveInterrupts);
  // Something that just happened still outranks it — a failed sweep is news, this is standing.
  assert.equal(barNoteOf({ dirty: true, syncing: true, notice: "x" }), "x");
  // Neither half alone: with nothing staged `Save` is disabled, and with nothing running there is
  // nothing to interrupt.
  assert.equal(barNoteOf({ dirty: true, syncing: false }), SETTINGS.saveNote);
  assert.equal(barNoteOf({ dirty: false, syncing: true }), SETTINGS.saveNote);
});

test("the bar's shape carries the sentence, so a moving number rebuilds it", () => {
  const one = settingsBarShape({ dirty: true, cost: SETTINGS.ruleRemovedCost(2, 100) });
  const two = settingsBarShape({ dirty: true, cost: SETTINGS.ruleRemovedCost(3, 100) });
  assert.notEqual(one, two);
  assert.notEqual(settingsBarShape({ dirty: false }), settingsBarShape({ dirty: true }));
  assert.notEqual(settingsBarShape({ saving: true }), settingsBarShape({ saving: false }));
  // The second slot's label and handler change with it (#320): `Discard changes`, or the retry a
  // failed restart leaves behind.
  assert.notEqual(settingsBarShape({ restartFailed: "boom" }), settingsBarShape({ restartFailed: null }));
});

test("a restart that failed keeps a way to try again, and nothing else does", () => {
  // #320. The save wrote the file and the daemon is still on the old settings — `Save` is disabled
  // by then (nothing staged), so this button is the only way out of the state from inside the app.
  assert.equal(barActionOf({ restartFailed: "the daemon did not stop within 8s" }), "restart");
  // A save that went through leaves no action: the restart already happened.
  assert.equal(barActionOf({ note: SETTINGS.savedRestarted }), "discard");
  assert.equal(barActionOf({ note: SETTINGS.savedNotRunning }), "discard");
  // …and a staged change is a change to discard, whatever the last save did.
  assert.equal(barActionOf({ restartFailed: "boom", dirty: true }), "discard");
  assert.equal(barActionOf({}), "discard");
});
