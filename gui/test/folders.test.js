// The folders model (#102 phase 5c-2): the decisions behind the Settings list, the add dialog and the
// removal confirmation. Pure, so every one of them is a function call; the page-level behaviour — the order of
// the commands, what is on screen while they run — is `fidelity:pairs`'s.

import { test } from "node:test";
import assert from "node:assert/strict";
import {
  BUSY_PHASES,
  WAIT_LIMIT,
  accountOf,
  addKeyOf,
  addRequestOf,
  addViewOf,
  blankAdd,
  listRows,
  priceOf,
  remotePathForProbe,
  removalOf,
  replyAbout,
  settledLinesOf,
  waitStepOf,
  withRule,
} from "../src/js/folders.js";
import { FOLDERS } from "../src/js/ui/copy.js";
import { barNoteOf, restartsEveryFolder, settingsBarShape } from "../src/js/screens/settings.js";

/** A flow that has been typed into and checked clean for exactly what it holds. */
function checked(over = {}) {
  const flow = { ...blankAdd(), name: "photos", local: "~/Photos", remote: "/Drive/Photos", ...over };
  const key = addKeyOf(flow);
  return {
    ...flow,
    pre: {
      suggested_name: "photos",
      name_error: null,
      refusal: null,
      surviving_index: null,
      warnings: [],
      ...(over.pre ?? {}),
    },
    preKey: key,
    checkedKey: key,
    ...over,
  };
}

// ---- the add dialog's button -------------------------------------------------------------------------

test("a_check_is_about_the_inputs_it_was_made_for", () => {
  const flow = checked();
  assert.equal(addViewOf(flow).primary, "add", "checked for exactly this text");
  // One character later, the same flow is not checked: the button is `Check folders` again.
  for (const field of ["name", "local", "remote"]) {
    const edited = { ...flow, [field]: `${flow[field]}x` };
    assert.equal(addViewOf(edited).primary, "check", `editing ${field} must put the check back`);
    assert.equal(addViewOf(edited).checked, false);
  }
  // Rules ride along with the add and are not what was checked.
  assert.equal(addViewOf({ ...flow, rules: ["*.psd"] }).primary, "add");
});

test("an_engine_answer_for_old_text_claims_nothing_about_the_new", () => {
  // The name field shows no error, and the button is not `Add folder`, between a keystroke and the answer.
  const flow = checked({ pre: { name_error: "the engine says no" } });
  assert.equal(addViewOf(flow).nameError, "the engine says no");
  const typedOn = { ...flow, name: "photo", nameTouched: true };
  assert.equal(addViewOf(typedOn).nameError, null);
  assert.equal(addViewOf(typedOn).blocked, false);
});

test("a_refused_name_blocks_the_check_and_is_the_engines_sentence", () => {
  const sentence = "two `[[pair]]` tables are named `docs`";
  const flow = checked({ name: "docs", nameTouched: true, checkedKey: null, pre: { name_error: sentence } });
  const view = addViewOf(flow);
  assert.equal(view.nameError, sentence);
  assert.equal(view.primary, "check");
  assert.equal(view.primaryEnabled, false);
  // The refusal of the rest speaks only when the name passed.
  const both = addViewOf(checked({ checkedKey: null, pre: { name_error: sentence, refusal: "other" } }));
  assert.equal(both.refusal, null);
});

test("a_name_error_waits_for_a_name_to_be_typed", () => {
  // A dialog that opens empty must not open with the engine's sentence about an empty name.
  const flow = {
    ...blankAdd(),
    pre: { name_error: "a `[[pair]]` table has an empty `name`" },
    preKey: addKeyOf(blankAdd()),
  };
  assert.equal(addViewOf(flow).nameError, null);
  assert.equal(addViewOf({ ...flow, nameTouched: true }).nameError, "a `[[pair]]` table has an empty `name`");
});

test("check_needs_a_filled_form_and_nothing_refused", () => {
  const empty = addViewOf(blankAdd());
  assert.equal(empty.primary, "check");
  assert.equal(empty.primaryEnabled, false);
  const filled = { ...blankAdd(), name: "p", local: "~/P", remote: "/Drive/P" };
  assert.equal(addViewOf(filled).primaryEnabled, true);
  assert.equal(addViewOf({ ...filled, remote: "  " }).primaryEnabled, false);
  assert.equal(addViewOf({ ...filled, phase: "checking" }).primaryEnabled, false);
});

test("a_dialog_with_something_in_flight_is_busy_and_cannot_be_left", () => {
  for (const phase of BUSY_PHASES) {
    const view = addViewOf(checked({ phase }));
    assert.equal(view.busy, true, phase);
    assert.equal(view.closable, false, phase);
    assert.equal(view.primary, "busy", phase);
  }
  for (const phase of ["form", "checking", "added", "unresolved"]) {
    assert.equal(addViewOf(checked({ phase })).closable, true, phase);
  }
  assert.equal(addViewOf(checked({ phase: "added" })).primary, "done");
});

test("the_survivor_and_the_warnings_belong_to_the_confirmation", () => {
  const survivor = { path: "~/P/.sync/sync_index.db", set_aside_pending: false, message: "m" };
  const flow = checked({ pre: { surviving_index: survivor, warnings: ["w"] } });
  assert.deepEqual(addViewOf(flow).survivor, survivor);
  assert.deepEqual(addViewOf(flow).warnings, ["w"]);
  // Before the check passes they are not said: the index is the CONFIRMATION's sentence.
  const unchecked = { ...flow, checkedKey: null };
  assert.equal(addViewOf(unchecked).survivor, null);
  assert.deepEqual(addViewOf(unchecked).warnings, []);
});

// ---- what is sent and measured -------------------------------------------------------------------------

test("the_request_trims_the_roots_and_carries_the_staged_rules_in_order", () => {
  const flow = {
    ...blankAdd(),
    local: "  ~/Photos ",
    remote: " /Drive/Photos  ",
    rules: ["*.psd", "tmp/**"],
  };
  assert.deepEqual(addRequestOf(flow), {
    local_root: "~/Photos",
    remote_root: "/Drive/Photos",
    exclude: ["*.psd", "tmp/**"],
  });
  // A copy: the dialog's list is not the request's.
  assert.notEqual(addRequestOf(flow).exclude, flow.rules);
});

test("a_rule_is_added_once_and_a_blank_one_not_at_all", () => {
  assert.deepEqual(withRule([], " *.psd "), ["*.psd"]);
  assert.deepEqual(withRule(["*.psd"], "*.psd"), ["*.psd"]);
  assert.deepEqual(withRule(["a"], "   "), ["a"]);
});

test("a_remote_path_is_probed_as_an_absolute_drive_path", () => {
  assert.equal(remotePathForProbe("/Drive/Photos"), "/Drive/Photos");
  assert.equal(remotePathForProbe(" Drive/Photos "), "/Drive/Photos");
});

test("a_price_is_exact_when_whole_and_a_floor_when_it_is_not", () => {
  assert.equal(
    priceOf("local", { files: 1204, bytes: 3_400_000_000, truncated: false, unreadable_directories: 0 }).text,
    "1,204 files, 3.4 GB",
  );
  // The Proton side has no size, and says only how many.
  assert.equal(priceOf("remote", { files: 1190, bytes: null, truncated: false }).text, "1,190 files");
  // A walk that stopped at its bound is a lower bound, not a total.
  assert.equal(priceOf("remote", { files: 600, bytes: null, truncated: true }).text, "at least 600 files");
  // A local walk that could not read a directory says so, and drops the size it can no longer vouch for.
  assert.equal(
    priceOf("local", { files: 10, bytes: 99, truncated: false, unreadable_directories: 2 }).text,
    "at least 10 files",
  );
  const failed = priceOf("remote", { error: "the daemon is busy" });
  assert.equal(failed.failed, true);
  assert.equal(failed.text, FOLDERS.add.notMeasured("the daemon is busy"));
  assert.equal(priceOf("local", null), null);
});

// ---- the wait for the daemon -----------------------------------------------------------------------------

test("the_folder_is_selected_only_once_the_daemon_lists_it", () => {
  assert.equal(waitStepOf({ name: "photos", listed: ["docs"], waits: 0, limit: WAIT_LIMIT }), "wait");
  assert.equal(
    waitStepOf({ name: "photos", listed: ["docs", "photos"], waits: 0, limit: WAIT_LIMIT }),
    "select",
  );
  assert.equal(
    waitStepOf({ name: "photos", listed: ["docs"], waits: WAIT_LIMIT, limit: WAIT_LIMIT }),
    "timeout",
  );
  // Listed on the last poll counts as listed: the wait is only over when the name is still missing.
  assert.equal(
    waitStepOf({ name: "photos", listed: ["photos"], waits: WAIT_LIMIT, limit: WAIT_LIMIT }),
    "select",
  );
  // Byte-exact: a folder called `Photos` is not `photos`.
  assert.equal(waitStepOf({ name: "photos", listed: ["Photos"], waits: 0, limit: WAIT_LIMIT }), "wait");
});

test("a_reply_about_another_folder_says_nothing_about_this_ones_merge", () => {
  const docs = { pair: "docs", reconcile_seq: 99 };
  const photos = { pair: "photos", reconcile_seq: 2 };
  assert.equal(replyAbout(docs, "photos"), null, "the folder that was selected before has its own counter");
  assert.equal(replyAbout(photos, "photos"), photos);
  assert.equal(replyAbout(null, "photos"), null);
  // A reply that names no folder (a daemon that predates the list) cannot be vouched for either.
  assert.equal(replyAbout({ reconcile_seq: 5 }, "photos"), null);
});

// ---- the list ------------------------------------------------------------------------------------------

const ROSTER = [
  { name: "docs", local_root: "~/Docs", remote_root: "/Drive/Docs" },
  { name: "photos", local_root: "~/Photos", remote_root: "/Drive/Photos" },
  { name: "music", local_root: "~/Music", remote_root: "/Drive/Music" },
];

test("the_list_is_the_files_in_the_files_order_with_the_daemons_words", () => {
  const rows = listRows({
    roster: ROSTER,
    pairs: [{ name: "photos" }, { name: "docs" }],
    pairStates: [
      { name: "photos", state: "running" },
      { name: "docs", state: "idle" },
    ],
    selected: "docs",
  });
  assert.deepEqual(
    rows.map((row) => [row.name, row.selected, row.word]),
    [
      ["docs", true, "up to date"],
      ["photos", false, "syncing"],
      // The file lists it and the daemon does not run it: the list says so rather than guess a state.
      ["music", false, FOLDERS.list.notRunning],
    ],
  );
  assert.equal(rows[0].local, "~/Docs", "the paths are the file's own text");
});

test("a_stopped_daemon_runs_nothing_so_every_row_says_what_the_chip_says", () => {
  const rows = listRows({ roster: ROSTER, pairs: [], pairStates: [], selected: "docs", reachable: false });
  assert.deepEqual(
    rows.map((row) => row.word),
    ["unreachable", "unreachable", "unreachable"],
    "the selector's own rule: a stopped daemon is the chip's word on every row, never `up to date` and never `not running yet`",
  );
});

test("a_list_without_a_roster_falls_back_to_the_daemons_folders", () => {
  const rows = listRows({
    roster: [],
    pairs: [{ name: "a", local_root: "/a", remote_root: "/Drive/a" }],
    pairStates: [{ name: "a", state: "idle" }],
    selected: "a",
  });
  assert.deepEqual(rows[0], {
    name: "a",
    local: "/a",
    remote: "/Drive/a",
    selected: true,
    word: "up to date",
  });
});

// ---- the removal confirmation --------------------------------------------------------------------------

test("removing_the_first_folder_names_the_new_default", () => {
  const removal = removalOf({ name: "docs", roster: ROSTER, setAsideDir: "/state/removed-pairs" });
  assert.equal(removal.first, true);
  assert.equal(removal.newDefault, "photos", "the next in FILE order, which is the one the engine promotes");
  assert.deepEqual(removal.lines, [
    "Syncing stops for docs.",
    "Nothing is deleted on this computer or in Proton Drive.",
    "Its sync history is moved aside to /state/removed-pairs, so adding the folder back later starts fresh.",
    "photos becomes the default folder: commands that name no folder, and older versions of this app, will mean it.",
  ]);
});

test("removing_another_folder_says_three_things_and_not_a_new_default", () => {
  const removal = removalOf({ name: "photos", roster: ROSTER, setAsideDir: "/state/removed-pairs" });
  assert.equal(removal.first, false);
  assert.equal(removal.newDefault, null);
  assert.equal(removal.lines.length, 3);
  assert.ok(!removal.lines.join(" ").includes("default folder"));
});

test("the_confirmation_does_not_promise_a_place_the_app_does_not_have", () => {
  // No state directory: the sentence names the place by what it is, and the answer says what happened.
  const removal = removalOf({ name: "docs", roster: ROSTER, setAsideDir: null });
  assert.ok(removal.lines[2].includes("the app's state folder"));
});

test("the_last_folder_cannot_be_removed", () => {
  const removal = removalOf({ name: "docs", roster: [ROSTER[0]], setAsideDir: null });
  assert.equal(removal.last, true);
  assert.equal(removalOf({ name: "ghost", roster: ROSTER }).known, false);
});

test("the_last_folder_says_why_it_cannot_be_removed", () => {
  // The confirmation of the only folder says WHY its button is disabled (review of #450, F5), and says
  // none of the promises that are about a removal that is not going to happen.
  const removal = removalOf({ name: "docs", roster: [ROSTER[0]], setAsideDir: "/state/removed-pairs" });
  assert.deepEqual(removal.lines, [FOLDERS.remove.last("docs")]);
  assert.match(FOLDERS.remove.last("docs"), /only folder/);
  assert.ok(!removal.lines.join(" ").includes("Syncing stops"));
  // With another folder beside it, the usual lines.
  assert.equal(removalOf({ name: "docs", roster: ROSTER, setAsideDir: "/s" }).lines.length, 4);
});

test("the_answer_is_the_commands_own_account_after_one_line_of_ours", () => {
  const lines = accountOf(
    {
      set_aside: {
        outcome: "pending",
        message: "The sync history of 'docs' was NOT moved yet: it is locked.",
      },
      settled_earlier: [{ message: "An earlier removal of 'old' finished." }, {}],
    },
    { name: "docs", restartNote: "Saved, but the sync service did not restart — x." },
  );
  assert.deepEqual(lines, [
    "docs was removed from the settings.",
    "The sync history of 'docs' was NOT moved yet: it is locked.",
    "An earlier removal of 'old' finished.",
    "Saved, but the sync service did not restart — x.",
  ]);
  // A reply with no account still says the one thing that is always true of an answer.
  assert.deepEqual(accountOf({}, { name: "docs" }), ["docs was removed from the settings."]);
});

test("an_add_that_finished_an_earlier_removal_says_so_in_the_removals_words", () => {
  // `add_pair` settles what an earlier removal could not finish, and its reply carries the account
  // (review of #450, F9). The same lines, from the same reader, as the removal dialog's answer.
  const reply = { settled_earlier: [{ message: "A" }, {}, { message: "" }, { message: "B" }] };
  assert.deepEqual(settledLinesOf(reply), ["A", "B"]);
  assert.deepEqual(settledLinesOf({}), []);
  assert.deepEqual(settledLinesOf(undefined), []);
  assert.deepEqual(accountOf(reply, { name: "x" }).slice(1), ["A", "B"]);
});

test("a_listed_folder_the_dialog_has_something_to_say_about_is_a_settled_dialog", () => {
  // The phase the add dialog rests in when the folder is listed and an earlier removal was finished by
  // the add: nothing is in flight, so it may be left, and the one button is `Done`.
  const view = addViewOf({ ...checked(), phase: "listed" });
  assert.equal(view.primary, "done");
  assert.equal(view.settled, true);
  assert.equal(view.busy, false);
  assert.equal(view.closable, true);
  assert.ok(!BUSY_PHASES.includes("listed"));
});

// ---- the save, with two folders or more ------------------------------------------------------------------

test("the_save_consequence_names_every_folder", () => {
  const staged = { configStaged: true, folderCount: 2 };
  assert.equal(restartsEveryFolder(staged), true);
  // TWO ARE `both`: "all two folders" is not a sentence anyone says (review of #450, F6), and the drawn
  // frame (`8a Save two folders`) says `both`.
  assert.equal(
    barNoteOf(staged),
    "Saving restarts syncing for both folders, briefly. Anything running now stops, and folders you paused stay paused.",
  );
  // From three, `all`, with the number in words up to ten and in digits above it (`cardinal(n, "mid")`).
  assert.ok(barNoteOf({ configStaged: true, folderCount: 3 }).includes("for all three folders, briefly"));
  assert.ok(barNoteOf({ configStaged: true, folderCount: 10 }).includes("for all ten folders, briefly"));
  assert.ok(barNoteOf({ configStaged: true, folderCount: 11 }).includes("for all 11 folders, briefly"));
  for (const n of [2, 3, 10, 11]) {
    assert.ok(!barNoteOf({ configStaged: true, folderCount: n }).includes("all two"), `${n}`);
  }
});

test("the_save_consequence_is_the_old_note_at_one_folder_and_when_nothing_restarts", () => {
  assert.equal(restartsEveryFolder({ configStaged: true, folderCount: 1 }), false);
  assert.equal(barNoteOf({ configStaged: true, folderCount: 1 }), barNoteOf({}));
  // A policy-only save writes gui.toml and restarts nothing: the sentence would be false.
  assert.equal(restartsEveryFolder({ configStaged: false, folderCount: 4 }), false);
  assert.ok(!barNoteOf({ configStaged: false, folderCount: 4 }).includes("restarts syncing"));
});

test("the_save_consequence_follows_a_cost_line_and_yields_to_a_notice", () => {
  const props = {
    configStaged: true,
    folderCount: 2,
    cost: "One rule removed — 2 files, 3.1 GB will start syncing.",
  };
  assert.ok(barNoteOf(props).startsWith(props.cost));
  assert.ok(barNoteOf(props).endsWith("folders you paused stay paused."));
  assert.equal(barNoteOf({ ...props, notice: "Saving…" }), "Saving…");
});

test("the_bar_is_rebuilt_when_the_sentence_appears", () => {
  // `settingsBarShape` is what tells the shell to rebuild the bar; a sentence that was not in it would leave
  // the old bar on screen after the second folder arrived.
  const before = settingsBarShape({ dirty: true, configStaged: true, folderCount: 1 });
  const after = settingsBarShape({ dirty: true, configStaged: true, folderCount: 2 });
  assert.notEqual(before, after);
});
