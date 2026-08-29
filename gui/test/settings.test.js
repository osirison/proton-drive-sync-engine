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
  disposalOf,
  policyCopyFor,
  policyOf,
  ruleEffect,
  totalLine,
  removalCost,
  configUpdate,
  isDirty,
  refusalReason,
  formatSchedule,
  parseSchedule,
  stepTimeOfDay,
  barActionOf,
  barNoteOf,
  settingsBarShape,
  restartEndingOf,
  restartUnresolved,
  clearsRestartFailure,
  saveNoteFor,
} from "../src/js/screens/settings.js";
import { SETTINGS } from "../src/js/ui/copy.js";
import { readFileSync } from "node:fs";
import { fileURLToPath } from "node:url";

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

test("the policy cards stop claiming a permanence the daemon will not deliver", () => {
  // `deletion_policy`'s wording was written on an identity this product no longer has: a local
  // deletion was ALWAYS permanent, so "permanent" and "on this computer" named one thing. Under the
  // default they do not, and two of the three sub-lines asserted outcomes that cannot happen.
  assert.equal(policyCopyFor("permanent").never.body, SETTINGS.askNeverSub);
  assert.equal(policyCopyFor("permanent").only_permanent.body, SETTINGS.askPermanentSub);
  assert.equal(policyCopyFor("trash").never.body, SETTINGS.askNeverTrashSub);
  assert.equal(policyCopyFor("trash").only_permanent.body, SETTINGS.askPermanentTrashSub);
  // Neither trash-mode line may promise permanence.
  for (const body of [SETTINGS.askNeverTrashSub, SETTINGS.askPermanentTrashSub]) {
    assert.doesNotMatch(body, /permanent/i, `"${body}" still claims permanence`);
  }
  // Unknown or not-yet-read: the OVER-warning wording, not the reassuring one.
  for (const unknown of [null, undefined, "", "shredded"]) {
    assert.equal(policyCopyFor(unknown).never.body, SETTINGS.askNeverSub);
  }
  // The card that was always true either way is untouched, and so are all three titles.
  assert.equal(policyCopyFor("trash").ask_every_time.body, SETTINGS.askEverySub);
  for (const mode of ["trash", "permanent"]) {
    assert.equal(policyCopyFor(mode).only_permanent.title, SETTINGS.askPermanent);
    assert.equal(policyCopyFor(mode).never.tone, "destructive");
  }
});

test("the disposal cards are their own setting, defaulting to the recoverable one", () => {
  // TWO SETTINGS ON ONE TAB, and this is the half that decides what a deletion DOES. An untouched
  // config says nothing about it and must draw `trash` — every install that existed before this key
  // is in exactly that state, and drawing `permanent` there would tell them their files are being
  // unlinked when the daemon has started trashing them.
  assert.equal(disposalOf({}), "trash");
  assert.equal(disposalOf({ local_delete_mode: "trash" }), "trash");
  assert.equal(disposalOf({ local_delete_mode: "permanent" }), "permanent");

  // No config read yet is not the same as a config that says nothing — the screen draws no
  // selection at all rather than guessing, exactly as `policyOf` does.
  assert.equal(disposalOf(null), null);

  // A value this build has never heard of draws NO card rather than the nearest one, so the next
  // save cannot silently rewrite a setting nobody touched. (The daemon refuses to start on such a
  // file; the screen still has to render one.)
  assert.equal(disposalOf({ local_delete_mode: "shredded" }), null);

  // And the two settings are independent: choosing a guard says nothing about the disposal.
  assert.equal(disposalOf({ deletion_policy: "never" }), "trash");
  assert.equal(policyOf({ local_delete_mode: "permanent" }), "ask_every_time");
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

// ---- the full-sweep schedule (#193) -----------------------------------------------------------

test("a schedule round-trips through the one spelling the file holds", () => {
  for (const text of [
    "weekly sun 03:00",
    "monthly day 15, 03:00",
    "weekly mon 00:00",
    "monthly day 31, 23:30",
  ]) {
    assert.equal(formatSchedule(parseSchedule(text)), text, text);
  }
  assert.deepEqual(parseSchedule("weekly sun 03:00"), { kind: "weekly", day: 6, at: "03:00" });
  assert.deepEqual(parseSchedule("monthly day 15, 03:00"), { kind: "monthly", day: 15, at: "03:00" });
});

test("anything the daemon would refuse reads as no schedule at all", () => {
  // The two parsers have to agree about what the file says: a value this accepted and
  // `src/schedule.rs` refused would draw a schedule belonging to a daemon that will not start.
  for (const text of [
    undefined,
    null,
    "",
    "weekly sunday 03:00",
    "weekly Sun 03:00",
    "weekly sun 3:00",
    "weekly sun 24:00",
    "weekly sun 03:60",
    "monthly day 0, 03:00",
    "monthly day 32, 03:00",
    "monthly day 15 03:00",
    "every sunday at 3am",
  ]) {
    assert.equal(parseSchedule(text), null, JSON.stringify(text));
  }
});

test("the time stepper wraps at midnight rather than clamping", () => {
  assert.equal(stepTimeOfDay("03:00", 1), "03:30");
  assert.equal(stepTimeOfDay("03:00", -1), "02:30");
  // A time of day is a circle: `00:00` has a step below it and `23:30` has one above it. Clamping
  // would make midnight a floor a user could reach and not leave.
  assert.equal(stepTimeOfDay("00:00", -1), "23:30");
  assert.equal(stepTimeOfDay("23:30", 1), "00:00");
});

test("the month-edge note names the day it is about, and only for a day a month can lack", () => {
  // PINNED HERE BECAUSE THE COPY GATE CANNOT SEE IT. `monthEdgeNote` is a template, and `walk` in
  // copy-gate.mjs collects strings only — so turning a checked constant into a template drops it out
  // of the gate silently (measured: the drawn-string total went 340 → 339 across that change). The
  // gate's own comments record the same hole for `PLAN.destructiveLocal` and `.destructiveMany` and
  // the same remedy: pin it in a unit test. Filed as #372 — 47 of the deck's 118 templates are in
  // neither of that gate's tables.
  assert.equal(SETTINGS.monthEdgeNote(31), "Months without a 31st are skipped to the last day.");
  assert.equal(SETTINGS.monthEdgeNote(30), "Months without a 30th are skipped to the last day.");
  // The teens rule, which is the whole reason the suffix is computed rather than looked up by last
  // digit — and 13 is inside the 1..31 range this is used over.
  assert.equal(SETTINGS.monthEdgeNote(13), "Months without a 13th are skipped to the last day.");
  assert.equal(SETTINGS.monthEdgeNote(1), "Months without a 1st are skipped to the last day.");
  assert.equal(SETTINGS.monthEdgeNote(22), "Months without a 22nd are skipped to the last day.");
  // And the shape the frame draws, which is FALSE about its own drawn value — every month has a
  // 15th. The screen shows this only for 29, 30 and 31; the string still has to render correctly,
  // because the reason it is not shown is a judgement about truth and not about grammar.
  assert.equal(SETTINGS.monthEdgeNote(15), "Months without a 15th are skipped to the last day.");
});

test("an absent schedule and a cleared one are the same staged state", () => {
  // Picking a day and unpicking it must land back on "no change" rather than on a staged edit that
  // would save nothing — `""` is how the screen spells the absence `write_config` clears the key on.
  assert.deepEqual(configUpdate({}, { full_scan_schedule: "" }), {});
  assert.deepEqual(configUpdate({ full_scan_schedule: "weekly sun 03:00" }, { full_scan_schedule: "" }), {
    full_scan_schedule: "",
  });
  assert.deepEqual(configUpdate({}, { full_scan_schedule: "weekly sun 03:00" }), {
    full_scan_schedule: "weekly sun 03:00",
  });
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
  assert.equal(barNoteOf({ configStaged: true, countedSync: true }), SETTINGS.saveInterrupts);
  // NOT HIDDEN BY THE COST LINE, which is the one ordering this decides: the cost describes what a
  // staged change lets through once saved, this describes what the click does to a transfer that is
  // happening now.
  assert.equal(barNoteOf({ configStaged: true, countedSync: true, cost: "c" }), SETTINGS.saveInterrupts);
  // Something that just happened still outranks it — a failed sweep is news, this is standing.
  assert.equal(barNoteOf({ configStaged: true, countedSync: true, notice: "x" }), "x");
  // Neither half alone: with nothing staged there is nothing to restart, and with nothing running
  // there is nothing to interrupt.
  assert.equal(barNoteOf({ configStaged: true, countedSync: false }), SETTINGS.saveNote);
  assert.equal(barNoteOf({ configStaged: false, countedSync: true }), SETTINGS.saveNote);
});

test("the interrupt warning is about a config change and a counted pass, not about `dirty`", () => {
  // #335. BOTH halves used to be a neighbouring question, and each drew this sentence over a save
  // that would interrupt nothing.
  //
  // `dirty` includes a staged notification POLICY — a `gui.toml` key the daemon has never heard of
  // — while the save gates its restart on daemon-config keys. Two definitions of one question, and
  // the screen believed the wrong one.
  assert.equal(
    barNoteOf({ dirty: true, configStaged: false, countedSync: true }),
    SETTINGS.saveNote,
    "a staged notification policy restarts nothing, so it interrupts nothing",
  );
  // And the other half: a plan-only rehearsal claims `syncing` (it must, or `activity` is gated off
  // every status reply), so the raw flag named a sync that was not running.
  assert.equal(
    barNoteOf({ configStaged: true, syncing: true, countedSync: false }),
    SETTINGS.saveNote,
    "a plan rehearsal is not the sync this sentence promises to stop",
  );
});

test("the bar's shape carries the sentence, so a moving number rebuilds it", () => {
  const one = settingsBarShape({ dirty: true, cost: SETTINGS.ruleRemovedCost(2, 100) });
  const two = settingsBarShape({ dirty: true, cost: SETTINGS.ruleRemovedCost(3, 100) });
  assert.notEqual(one, two);
  assert.notEqual(settingsBarShape({ dirty: false }), settingsBarShape({ dirty: true }));
  assert.notEqual(settingsBarShape({ saving: true }), settingsBarShape({ saving: false }));
  // The second slot's label and handler change with it (#320): `Discard changes`, or the retry an
  // unresolved restart leaves behind.
  assert.notEqual(
    settingsBarShape({ restartEnding: "never_stopped" }),
    settingsBarShape({ restartEnding: null }),
  );
  // …and it moves with the OTHER half of that decision too (#335): the same ending yields the slot
  // once a daemon-config change is staged, so a shape reading only the ending would leave the bar
  // on screen with the wrong label.
  assert.notEqual(
    settingsBarShape({ restartEnding: "never_stopped", configStaged: true }),
    settingsBarShape({ restartEnding: "never_stopped", configStaged: false }),
  );
});

test("a restart that left something wrong keeps a way to try again, and nothing else does", () => {
  // #320. The save wrote the file and the daemon is still on the old settings — `Save` is disabled
  // by then (nothing staged), so this button is the only way out of the state from inside the app.
  assert.equal(barActionOf({ restartEnding: "never_stopped" }), "restart");
  assert.equal(barActionOf({ restartEnding: "not_started" }), "restart");
  // A save that went through leaves no action: the restart already happened.
  assert.equal(barActionOf({ restartEnding: null }), "discard");
  assert.equal(barActionOf({}), "discard");
  // …and a staged DAEMON-CONFIG change is a change to discard, because saving again restarts again.
  assert.equal(barActionOf({ restartEnding: "never_stopped", configStaged: true }), "discard");
});

// ---- the restart's five endings (#335) ---------------------------------------------------------

test("each ending gets its own sentence, and two of them are opposites", () => {
  // THE BUG THIS FIXES. #328 typed two endings and let three collapse into one `Err(String)`, so
  // the screen said `It is still running the old settings` about all three. That is true of
  // `never_stopped` — and the exact opposite of the truth for `not_started`, where the stop
  // SUCCEEDED and nothing is running at all. Reachable on any install that is not the systemd one.
  assert.equal(saveNoteFor("restarted"), SETTINGS.savedRestarted);
  assert.equal(saveNoteFor("not_running"), SETTINGS.savedNotRunning);
  assert.equal(saveNoteFor("not_started", "ENOENT"), SETTINGS.savedNothingRunning("ENOENT"));
  assert.equal(saveNoteFor("never_stopped", "8s"), SETTINGS.savedOldSettings("8s"));
  assert.equal(saveNoteFor("undetermined", "no answer"), SETTINGS.savedUnknownState("no answer"));

  // The two sentences that must not be each other's.
  assert.match(saveNoteFor("never_stopped", "8s"), /still running the old settings/);
  assert.doesNotMatch(
    saveNoteFor("not_started", "ENOENT"),
    /still running the old settings/,
    "the stop succeeded — saying anything is still running is false in the dangerous direction",
  );
  assert.match(saveNoteFor("not_started", "ENOENT"), /Nothing is syncing/);
});

test("an ending this build cannot name claims nothing rather than falling through to `fine`", () => {
  // A fall-through arm that reads as success is #246's shape. `restartEndingOf` degrades an
  // unrecognised tag — a backend newer than the window, or an older one with no `ending` at all —
  // and the sentence for it asserts nothing about what is running.
  assert.equal(restartEndingOf({ ending: "reloaded_in_place" }), "unknown");
  assert.equal(restartEndingOf({ restarted: true }), "unknown", "#328's old shape is not an ending");
  assert.equal(restartEndingOf(null), "unknown");
  assert.equal(restartEndingOf({ ending: "restarted", detail: "x" }), "restarted");
  assert.equal(saveNoteFor("unknown"), SETTINGS.savedUnknownEnding);
  assert.doesNotMatch(saveNoteFor("unknown"), /running your new settings|isn't running/);
  // And it keeps the way out on screen, because nothing here proves the state is fine.
  assert.ok(restartUnresolved("unknown"));
});

test("only the two settled endings let the state be forgotten", () => {
  // The one definition of "there is still something to fix", read by the bar's retry slot, by what
  // survives navigating away, and by what an edit forgets. `restarted` and `not_running` are
  // acknowledgements of something finished; every other ending is a file on disk running ahead of
  // the service, which is still true two screens later.
  assert.equal(restartUnresolved("restarted"), false);
  assert.equal(restartUnresolved("not_running"), false);
  assert.equal(restartUnresolved(null), false, "no save at all is not an unresolved one");
  for (const ending of ["not_started", "never_stopped", "undetermined", "unknown"]) {
    assert.equal(restartUnresolved(ending), true, `${ending} still needs a restart`);
  }
});

/** A latch as `app.js` writes it: an ending, and the request clock it was written at. */
const latch = (ending, evidenceFloor = 7) => ({ ending, reason: "boom", evidenceFloor });
/** What a render knows: whether the socket answers, and which request said so. */
const seen = (socketAnswers, statusIssue = 8) => ({ socketAnswers, statusIssue });

test("the latch is re-validated against the daemon, and the two failures invert", () => {
  // #335. Nothing re-validated it, so systemd's `Restart=on-failure` brought the service up on the
  // NEW settings while the bar still offered to restart it.
  //
  // `not_started` was reached at a moment of CONFIRMED absence — the socket was authoritatively
  // empty and the start then failed — so a daemon answering LATER began after that moment and read
  // the file this save wrote. The state is over.
  assert.equal(clearsRestartFailure(latch("not_started"), seen(true)), true);
  assert.equal(clearsRestartFailure(latch("not_started"), seen(false)), false, "nothing listening");
  // `never_stopped` is the opposite: the daemon that is answering is the one that would NOT stop,
  // still on the settings it started with. A reachable socket is the problem, not the end of it.
  assert.equal(clearsRestartFailure(latch("never_stopped"), seen(true)), false);
  // And `undetermined` observed nothing, so it may not conclude anything from a later poll either.
  assert.equal(clearsRestartFailure(latch("undetermined"), seen(true)), false);
  assert.equal(clearsRestartFailure(latch("unknown"), seen(true)), false);
});

test("evidence older than the outcome may never retire it", () => {
  // THE BUG THIS PR'S OWN FIRST ATTEMPT SHIPPED WITH, found by the review of #338 — and it defeated
  // the deliverable in its own headline scenario, which is why it is worth this much test.
  //
  // The re-validation runs in `render()`, and `saveSettings` renders synchronously the instant
  // `restartForSave` records its answer — `poll()` having been fired and NOT awaited. So that render
  // sees the last COMPLETED poll. In the `not_started` case that answer is *necessarily* a
  // reachable daemon: the restart only stopped anything because the probe said it was running. The
  // latch was therefore nulled before it was ever drawn, nothing re-latches it, and
  // `Nothing is syncing right now.` plus `Restart it now` never appeared.
  //
  // A pure predicate over (ending, reachable) could not see this, and neither could a source-text
  // pin. The rule is about ORDER, so the test has to be about order.
  const outcome = latch("not_started", 7);
  assert.equal(
    clearsRestartFailure(outcome, seen(true, 7)),
    false,
    "the poll already in hand was issued before the restart finished — it cannot speak for it",
  );
  assert.equal(
    clearsRestartFailure(outcome, seen(true, 6)),
    false,
    "nor may an even older one, which is what an in-flight poll landing late carries",
  );
  // The poll the restart itself kicked off is the first that may.
  assert.equal(clearsRestartFailure(outcome, seen(true, 8)), true);
  // An answer carrying no request id is not newer than anything, so it speaks for nothing.
  assert.equal(clearsRestartFailure(outcome, { socketAnswers: true }), false);
  // A latch with NO floor is the old behaviour restored — cleared by the very first answer. It
  // cannot happen while there is one construction site (pinned below), and this is what that pin
  // is protecting rather than a shape the app can produce.
  assert.equal(clearsRestartFailure({ ending: "not_started" }, seen(true, 1)), true);
});

test("a staged notification policy does not take the retry slot away", () => {
  // #335. `dirty` was the predicate here, and a staged policy made it true — so the retry vanished
  // in favour of `Discard changes`, and the save it made room for writes `gui.toml` and restarts
  // nothing. In the two endings where the daemon is UP on the old settings this bar is the only
  // restart control left in the app, so that took the only way out and gave nothing back.
  assert.equal(barActionOf({ restartEnding: "never_stopped", dirty: true, configStaged: false }), "restart");
});

// ---- the two callers, pinned in app.js's source ------------------------------------------------

/** `app.js`'s body between `function <name>(` (async or not) and the next top-level `}`. */
function functionBody(source, name) {
  const start = source.search(new RegExp(`^(async )?function ${name}\\(`, "m"));
  assert.notEqual(start, -1, `app.js no longer has ${name} — if it moved, move this test with it`);
  const end = source.indexOf("\n}", start);
  assert.notEqual(end, -1, `${name} has no end`);
  return source.slice(start, end);
}

test("the save's restart is `onlyIfRunning` and the retry's is not", () => {
  // #335: NOTHING PINNED THIS. `api.restartService`'s parameter defaults to `false`, so dropping the
  // argument — `api.restartService(true)` → `api.restartService()` — silently reverts the save to
  // starting a daemon nobody asked to start, which is the #320 decision, and every JS and Rust test
  // stayed green. Reading the source is the only check available for a caller this suite cannot
  // execute (no DOM); `onboarding-latch.test.js` uses the same construction for the same reason.
  //
  // ANCHORED INSIDE EACH FUNCTION, not asked of the file: `restartService(true)` appearing anywhere
  // would pass with the two calls swapped, which is the mistake worth catching.
  const app = readFileSync(fileURLToPath(new URL("../src/js/app.js", import.meta.url)), "utf8");

  const save = functionBody(app, "restartForSave");
  assert.match(
    save,
    /api\.restartService\(true\)/,
    "a save must not start a service that was not running (#320)",
  );

  const retry = functionBody(app, "restartAfterSave");
  assert.match(retry, /api\.restartService\(\)/, "the retry has a stopped daemon to fix");
  assert.doesNotMatch(
    retry,
    /api\.restartService\(true\)/,
    "`Restart it now` after a start that failed must not decline because nothing is running",
  );

  // And #335's own omission: `barNoteOf` puts `notice` first, so a stale `Sweep now` failure masked
  // every one of the endings below it. The retry's sibling always cleared it; the save's did not.
  assert.match(save, /settingsNotice = null/, "a stale notice would mask the ending this save had");
});

test("the latch is only ever forgotten through the predicate that knows whether it is resolved", () => {
  // #335. `resetSettingsScreen` runs on every entry to the settings route and nulled the latch
  // outright, so walking Activity → Settings lost the amber line and `Restart it now` while the
  // state they describe — a file on disk running ahead of the service — was still true. Every
  // forgetting path now goes through `clearSaveOutcome`, which keeps an unresolved ending.
  const app = readFileSync(fileURLToPath(new URL("../src/js/app.js", import.meta.url)), "utf8");
  const lines = app.split("\n");
  const assignments = lines.filter((l) => /^\s*settingsSaveOutcome = null;/.test(l));
  assert.equal(
    assignments.length,
    1,
    "exactly one place may null it — `render`'s re-validation, which asks `clearsRestartFailure` first",
  );
  // AND ONE PLACE MAY WRITE IT, which is what keeps the evidence floor from being forgotten on one
  // of the two paths: a latch with no floor is cleared by the first poll that arrives, which is the
  // pre-review behaviour exactly.
  const writes = lines.filter((l) => /^\s*settingsSaveOutcome = \{/.test(l));
  assert.equal(writes.length, 1, "`latchRestart` is the only builder — see its own comment");
  assert.match(
    functionBody(app, "latchRestart"),
    /evidenceFloor: store\.select\.statusesIssued\(\)/,
    "the floor must be the request clock at the moment the outcome was recorded",
  );
  const reset = functionBody(app, "resetSettingsScreen");
  assert.match(reset, /clearSaveOutcome\(\)/, "navigation must ask, not null");
  assert.doesNotMatch(reset, /settingsSaveOutcome/, "…and it must not reach the state directly");
});
