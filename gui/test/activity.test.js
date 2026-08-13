// S5's model, and it is almost entirely UNDRAWN CODE.
//
// `7a File lookup` draws ONE of five verdicts. `6a Activity passes` draws ONE of four summary
// shapes and six of a possible fourteen counters. `7a Never synced` draws its sentence at two
// groups where the shipped screen can only ever source one. So the fidelity gate — which compares
// the app against the frames — cannot see most of what this screen does, and the copy gate cannot
// either: it compares the deck to the frames and never renders the app at all.
//
// That is the same hole S4 found and filled the same way (`plan.test.js` pins `PLAN.destructive*`,
// two templates no frame draws). This project's own record says undrawn states are where the bugs
// are — S1: 9, C1–C5: 14, S2: 5, S3: 11+4, none caught by any gate — so these are the assertions
// standing in for the frames that were never drawn.

import test from "node:test";
import assert from "node:assert/strict";

import {
  verdictOf,
  passesSummaryOf,
  neverSyncedFrom,
  footerVariantOf,
  elideId,
  normaliseQuery,
  searchOutcome,
  failureLabel,
  UNREACHABLE_NEEDLES,
} from "../src/js/screens/activity.js";
import { ACTIVITY } from "../src/js/ui/copy.js";
import { cardinal } from "../src/js/ui/format.js";

// ---- the lookup's five verdicts -------------------------------------------------------------

const tracked = (over = {}) => ({
  tracked: true,
  sync_status: "synced",
  entity_kind: "file",
  file_size: 1_200_000,
  mtime: 1_700_000_000,
  proton_id: "8b3c1f2a~4c8f2e7d10b64f2ca39c5e0b8d7f9a21",
  ...over,
});

test("a synced file is the drawn verdict, and its `since` clause needs a time", () => {
  assert.equal(verdictOf(tracked(), "14:32").title, ACTIVITY.lookup.safe);
  assert.equal(verdictOf(tracked(), "14:32").sub, ACTIVITY.lookup.safeSub("14:32"));
  assert.equal(verdictOf(tracked(), "14:32").mark, "settled");
  // No time, no claim about when. The sentence is `Identical … since X`; without an X there is
  // nothing true to put in it, so the clause goes rather than rendering an em-dash inside prose.
  assert.equal(verdictOf(tracked(), null).sub, null);
  assert.equal(verdictOf(tracked(), null).title, ACTIVITY.lookup.safe);
});

test("a modified file says the copy here is newer, and does not wear the settled mark", () => {
  const v = verdictOf(tracked({ sync_status: "modified" }), "14:32");
  assert.equal(v.title, ACTIVITY.lookup.changed);
  assert.equal(v.sub, ACTIVITY.lookup.changedSub);
  assert.notEqual(v.mark, "settled");
});

test("a conflicted file says both sides changed and nothing is lost", () => {
  const v = verdictOf(tracked({ sync_status: "conflict" }), "14:32");
  assert.equal(v.title, ACTIVITY.lookup.conflict);
  assert.match(v.sub, /Nothing is lost/);
  assert.notEqual(v.mark, "settled");
});

test("a path the index has never seen is a miss, with no mark at all", () => {
  for (const status of [null, undefined, { tracked: false }, { tracked: false, sync_status: null }]) {
    const v = verdictOf(status, "14:32");
    assert.equal(v.title, ACTIVITY.lookup.noMatch, `for ${JSON.stringify(status)}`);
    // A miss is not a state a file is IN, so it gets no hexagon. Drawing the settled mark over
    // "no file by that name" would put the safe mark on a file that does not exist.
    assert.equal(v.mark, null);
  }
});

test("a directory is answered as a folder rather than as a file that is fine", () => {
  const v = verdictOf(tracked({ entity_kind: "directory" }), "14:32");
  assert.equal(v.title, ACTIVITY.lookup.folder);
  assert.notEqual(v.mark, "settled");
});

// THE ARM NOBODY DESIGNED FOR. `path_sync_status` documents three values for `sync_status` and the
// engine could grow a fourth; the dangerous default is the reassuring one, so this pins that an
// unrecognised status never comes back as "safely on both sides".
test("an unrecognised sync_status fails closed — never the settled mark, never the safe words", () => {
  // `constructor` / `__proto__` / `toString` are in the list because the verdicts are a TABLE now,
  // and a table can be reached by a key off Object's prototype where the switch it replaced could
  // not. Each answers with something truthy that has no `mark` and no `sub`, so an unguarded
  // lookup throws rather than falling through to the cautious verdict. `sync_status` is a string
  // off the wire; nothing stops the engine sending one of these.
  for (const unknown of [
    "",
    "pending",
    "PENDING",
    "synced ",
    null,
    undefined,
    7,
    "constructor",
    "__proto__",
    "toString",
  ]) {
    const v = verdictOf(tracked({ sync_status: unknown }), "14:32");
    assert.notEqual(v.mark, "settled", `mark for ${JSON.stringify(unknown)}`);
    assert.notEqual(v.title, ACTIVITY.lookup.safe, `title for ${JSON.stringify(unknown)}`);
    // AND IT MUST NOT CLAIM ANY OTHER STATE EITHER. The first version withheld the settled mark and
    // then said "Changed here, not sent yet" — cautious about the hexagon, specific and possibly
    // false about the file. A status this build cannot read is a check it cannot report, so it
    // reports the failure and quotes the value.
    assert.notEqual(v.title, ACTIVITY.lookup.changed, `title for ${JSON.stringify(unknown)}`);
    assert.equal(v.title, ACTIVITY.lookup.failed, `title for ${JSON.stringify(unknown)}`);
    assert.equal(v.mark, null, `mark for ${JSON.stringify(unknown)}`);
    assert.match(v.error, /sync_status/, `error for ${JSON.stringify(unknown)}`);
  }
});

// ---- the passes summary ----------------------------------------------------------------------

const clean = (secs) => ({ epoch_secs: secs, last_error: null });
const failed = (secs) => ({ epoch_secs: secs, last_error: "proton-drive: connection timed out after 60s" });

test("the drawn summary renders from the drawn numbers", () => {
  // OLDEST FIRST, as the wire sends it — `daemon.rs` pushes each pass and drains the front.
  const entries = [...Array(19)].map((_, i) => clean(i)).concat([clean(19)]);
  entries[8] = failed(8);
  assert.equal(passesSummaryOf(entries), ACTIVITY.passes.summary(19, 20, 1, true));
});

test("no history at all has no sentence — the caller omits the line rather than claim zero passes", () => {
  assert.equal(passesSummaryOf([]), null);
});

test("every pass clean says so without the of-the-last construction", () => {
  const s = passesSummaryOf([clean(1), clean(2), clean(3)]);
  assert.equal(s, "All 3 recent passes finished cleanly.");
  assert.doesNotMatch(s, /failed/);
});

test("one clean pass is singular", () => {
  assert.equal(passesSummaryOf([clean(1)]), "All 1 recent pass finished cleanly.");
});

// `recovered` is the whole reason the summary takes four arguments: "retried on its own" is a claim
// about the ORDER of the history, not about any one entry, and it is false in precisely the state
// where a user most needs the truth.
test("a failure that is still the newest pass is NOT described as retried", () => {
  const s = passesSummaryOf([clean(1), clean(2), failed(3)]);
  assert.match(s, /The most recent one failed\./);
  assert.doesNotMatch(s, /retried/);
});

test("a failure with a later success is described as retried", () => {
  const s = passesSummaryOf([clean(1), failed(2), clean(3)]);
  assert.match(s, /retried on its own/);
  assert.doesNotMatch(s, /The most recent one failed/);
});

test("more than one failure agrees in number", () => {
  const s = passesSummaryOf([failed(1), failed(2), clean(3)]);
  assert.match(s, /Two failed and retried on their own\./);
});

// ---- the never-synced band ---------------------------------------------------------------------

const rule = (over = {}) => ({
  pattern: "*.tmp",
  files: 2,
  bytes: 2_940_000,
  unique_files: 2,
  unique_bytes: 2_940_000,
  samples: [],
  ...over,
});

test("no report and no matching rule both mean no band", () => {
  assert.equal(neverSyncedFrom(null), null);
  assert.equal(neverSyncedFrom({ rules: [], total_files: 0 }), null);
  // A rule that matched nothing is not a reason to tell someone files are never synced.
  assert.equal(neverSyncedFrom({ rules: [rule({ files: 0 })], total_files: 0 }), null);
});

// THE COUNT COMES OFF THE REPORT, not off the rules. `total_files` is documented as "distinct files
// hidden by at least one rule" — the union — and neither per-rule field can stand in for it:
// `files` double-counts a path two rules both hide, and `unique_files` is `matches == 1` and DROPS
// it. The first version of this summed `unique_files`, and the first version of this test asserted
// that it should.
test("the band's count is the report's union, not a sum of the rules", () => {
  const report = {
    rules: [rule({ files: 2, unique_files: 2 }), rule({ pattern: "*.bak", files: 3, unique_files: 1 })],
    total_files: 4,
  };
  assert.equal(neverSyncedFrom(report).total, 4);
  assert.equal(neverSyncedFrom(report).rules.length, 2);
});

// The failure mode that made summing `unique_files` unsafe rather than merely imprecise: on a
// machine where every excluded file matches two rules, every `unique_files` is 0 — so the band
// disappeared entirely while the files stayed unsynced.
test("files hidden by two rules still raise the band", () => {
  const report = {
    rules: [rule({ files: 2, unique_files: 0 }), rule({ pattern: "*.bak", files: 2, unique_files: 0 })],
    total_files: 2,
  };
  assert.equal(neverSyncedFrom(report).total, 2);
  assert.equal(neverSyncedFrom(report).rules.length, 2);
});

// A failed command and a path that is not in the index both leave `status` null. Telling someone
// their file is missing when the CHECK is what failed is the worst answer this screen can give.
test("a failed lookup says the check failed, and never that the file is not there", () => {
  const v = verdictOf(null, "14:32", "proton-syncd: index is locked");
  assert.equal(v.title, ACTIVITY.lookup.failed);
  assert.notEqual(v.title, ACTIVITY.lookup.noMatch);
  assert.equal(v.error, "proton-syncd: index is locked");
  assert.equal(v.mark, null);
  // The error outranks a reply, because a reply arriving beside an error cannot be trusted either.
  assert.equal(verdictOf(tracked(), "14:32", "boom").title, ACTIVITY.lookup.failed);
  // And no error means the ordinary paths are untouched.
  assert.equal(verdictOf(null, "14:32", null).title, ACTIVITY.lookup.noMatch);
});

// ---- the search's three outcomes -------------------------------------------------------------
//
// `search_files` replaced `path_sync_status` on this field (G21), and the screen's whole behaviour
// turns on how many files came back. Nothing draws two of the three.

const match = (path, over = {}) => ({ path, status: tracked(over) });

test("one match resolves straight to the file, under the path the index stores", () => {
  // The gap this closes: someone types `spec.md` and means `docs/spec.md`.
  const out = searchOutcome({ matches: [match("docs/spec.md")], total: 1, query: "spec.md" }, "spec.md");
  assert.equal(out.lookup.path, "docs/spec.md");
  assert.equal(out.lookup.status.sync_status, "synced");
  assert.equal(out.matches.total, 1);
});

test("several matches answer about none of them", () => {
  const out = searchOutcome(
    { matches: [match("a/notes.md"), match("b/notes.md")], total: 2, query: "notes.md" },
    "notes.md",
  );
  // A verdict about the wrong `notes.md` is worse than a question, so there is no lookup to draw.
  assert.equal(out.lookup, null);
  assert.equal(out.matches.matches.length, 2);
});

test("the count belongs to what was typed, not to the resolved path", () => {
  // A pasted absolute path resolves to a different string, and the field still holds the paste.
  const out = searchOutcome(
    { matches: [match("docs/spec.md")], total: 1, query: "docs/spec.md" },
    "~/ProtonDrive/docs/spec.md",
  );
  assert.equal(out.matches.typed, "~/ProtonDrive/docs/spec.md");
  assert.equal(out.matches.query, "docs/spec.md");
});

test("no match is a miss under the query the backend actually ran", () => {
  // `~` expanded and the sync root stripped happen in Rust, so the miss must name the reply's query
  // and not the typed one — otherwise the screen says no such file about a path it never asked for.
  const out = searchOutcome({ matches: [], total: 0, query: "docs/spec.md" }, "~/ProtonDrive/docs/spec.md");
  assert.equal(out.lookup.path, "docs/spec.md");
  assert.equal(out.lookup.status, null);
  assert.equal(verdictOf(out.lookup.status, "14:32", out.lookup.error).title, ACTIVITY.lookup.noMatch);
});

test("a failed search is a failure, not a miss", () => {
  // No reply at all — the command threw — so the error travels with the empty answer.
  const out = searchOutcome(null, "spec.md", "index is locked");
  assert.equal(out.lookup.error, "index is locked");
  assert.equal(verdictOf(out.lookup.status, "14:32", out.lookup.error).title, ACTIVITY.lookup.failed);
  assert.equal(out.matches.total, 0);
});

test("the count is the total, not the capped list", () => {
  // The cap is the screen's, not the answer's: saying `2 matches` when 132 files match would send
  // someone away believing their file is not there.
  const out = searchOutcome({ matches: [match("a/x.md"), match("b/x.md")], total: 132, query: "x" }, "x");
  assert.equal(out.matches.total, 132);
  assert.equal(out.matches.matches.length, 2);
});

// The `Can't be synced` group has no Phase-1 source (#232), so the live sentence always renders at
// `cannot: 0` — and `cardinal(0)` is `zero`, which would read as a sentence about a group nobody
// measured.
test("with no can't-be-synced group the sentence stops after the first clause", () => {
  const s = ACTIVITY.neverSyncedSub(2, 0);
  assert.equal(s, "They sit in your folder but aren't copied anywhere. Two match a rule you wrote.");
  assert.doesNotMatch(s, /zero/i);
  assert.doesNotMatch(s, /can't be synced at all/);
});

test("both groups present renders the drawn sentence, with the second clause lower-cased", () => {
  assert.equal(
    ACTIVITY.neverSyncedSub(2, 2),
    "They sit in your folder but aren't copied anywhere. Two match a rule you wrote; two can't be synced at all.",
  );
});

test("one file matching a rule agrees in number", () => {
  assert.match(ACTIVITY.neverSyncedSub(1, 0), /One matches a rule you wrote\./);
});

test("cardinal's mid register lower-cases the spelled forms and leaves digits alone", () => {
  assert.equal(cardinal(2), "Two");
  assert.equal(cardinal(2, "mid"), "two");
  assert.equal(cardinal(11, "mid"), "11");
});

// ---- the four templates that were ungrammatical at one --------------------------------------
//
// Every drawn instance of each of these is plural, so they were written with the plural baked in
// and rendered `1 files are never synced` the first time a live count reached them. All four states
// are reachable: one excluded file, one file moved, one day.

test("the counted sentences agree in number at one", () => {
  assert.equal(ACTIVITY.neverSyncedTitle(1), "1 file is never synced");
  assert.equal(ACTIVITY.neverSyncedTitle(4), "4 files are never synced");
  assert.equal(ACTIVITY.neverSyncedDialog.title(1), "1 file is never synced");
  assert.equal(ACTIVITY.lastToMoveSub(1, 1), "1 file in the last 1 day");
  assert.equal(ACTIVITY.lastToMoveSub(7, 3), "7 files in the last 3 days");
  assert.equal(ACTIVITY.allFiles(1), "All 1 file");
  assert.equal(ACTIVITY.allFiles(7), "All 7 files");
});

// ---- the rest of the model --------------------------------------------------------------------

test("the footer variant is per TAB, not per route", () => {
  assert.equal(footerVariantOf({ tab: "files" }), "tight");
  assert.equal(footerVariantOf({ tab: "passes" }), "standard");
  // The files tab is the default, including when the screen has not said which tab it is on.
  assert.equal(footerVariantOf({}), "tight");
  assert.equal(footerVariantOf(null), "tight");
});

test("the id elide takes the NODE half, because the volume half never changes", () => {
  assert.equal(elideId("8b3c1f2a~4c8f2e7d10b64f2ca39c5e0b8d7f9a21"), "4c8f…9a21");
  // Short enough to show whole, and a bare node id with no volume prefix.
  assert.equal(elideId("8b3c1f2a~abc123"), "abc123");
  assert.equal(elideId("4c8f2e7d10b64f2ca39c5e0b8d7f9a21"), "4c8f…9a21");
});

// The query and the answer have to agree on what "the same path" is, or the count never appears —
// app.js decides which path to ASK about and the screen decides whether the answer is still the
// one on screen, and the two read the same function to stay in step.
test("a typed query normalises to the path the index is asked for", () => {
  assert.equal(normaliseQuery("  docs/spec.md  "), "docs/spec.md");
  assert.equal(normaliseQuery("/docs/spec.md"), "docs/spec.md");
  assert.equal(normaliseQuery("///docs/spec.md"), "docs/spec.md");
  assert.equal(normaliseQuery(""), "");
  assert.equal(normaliseQuery(null), "");
  assert.equal(normaliseQuery(undefined), "");
});

// ---- what a failed pass is allowed to claim (#258) --------------------------------------------
//
// The row used to label EVERY failure `Couldn't reach Proton Drive`, on the sole test
// `last_error != null` — over a full disk, over a binary that had moved, and directly above the
// daemon's own words saying otherwise. These are the two directions that matters in, and they are
// not symmetric: a miss is quieter than the truth and still true, a false hit is the bug.

test("the drawn label survives, because the frame's own error is a connection timeout", () => {
  // Not a paraphrase: this is the exact string `6a Activity passes` draws and the fixture feeds.
  assert.equal(failureLabel(ACTIVITY.passes.exampleDaemonError), ACTIVITY.passes.unreachable);
});

test("a failure Proton had nothing to do with does not blame Proton", () => {
  for (const error of [
    "No space left on device (os error 28)",
    "failed to spawn proton-drive: No such file or directory (os error 2)",
    "permission denied reading /home/qp/ProtonDrive/notes.md",
    "session expired — sign in again",
    "refusing to bind control socket: /run/user/1000/proton-sync.sock already exists",
  ]) {
    assert.equal(failureLabel(error), ACTIVITY.passes.failed, error);
  }
});

test("the reach-shaped failures the engine and the CLI can actually produce all match", () => {
  for (const error of [
    // The engine's own, `src/proton.rs`: `proton-drive {operation} timed out after {duration}`.
    "proton-drive list timed out after 120s",
    "connection refused",
    "Connection reset by peer",
    "network is unreachable",
    "curl: (6) Could not resolve host: drive.proton.me",
    "temporary failure in name resolution",
    "no route to host",
  ]) {
    assert.equal(failureLabel(error), ACTIVITY.passes.unreachable, error);
  }
});

// A row whose error is missing or empty still has to render a label. It cannot claim a cause it
// does not have — and `String(null)` is the word `null`, which would have matched nothing and
// worked by accident; `String(undefined)` contains no needle either, but neither is a reason.
test("an absent error falls to the neutral label rather than throwing", () => {
  assert.equal(failureLabel(null), ACTIVITY.passes.failed);
  assert.equal(failureLabel(undefined), ACTIVITY.passes.failed);
  assert.equal(failureLabel(""), ACTIVITY.passes.failed);
});

// Case is the daemon's to choose and the CLI's to vary — `Connection reset` and `connection reset`
// are the same failure, and a classifier that reads one and not the other is a coin toss.
test("the match is case-insensitive", () => {
  assert.equal(failureLabel("CONNECTION TIMED OUT"), ACTIVITY.passes.unreachable);
  assert.equal(failureLabel("Network Is Unreachable"), ACTIVITY.passes.unreachable);
});

// A COUNT IS A CLAIM, and this branch already got one wrong: the list comment said "all three" of
// four errors, caught in review. DEVIATIONS §93 and the PR body both say NINE phrases, so the number
// is pinned here rather than left as prose that ages. Growing the list is fine; growing it without
// touching the sentence that quotes it is not.
test("the needle list is the nine phrases the write-up claims", () => {
  assert.equal(UNREACHABLE_NEEDLES.length, 9);
  // Lower-case, because the match lower-cases the message and not the needle — an upper-case entry
  // here would silently never fire.
  for (const needle of UNREACHABLE_NEEDLES) assert.equal(needle, needle.toLowerCase(), needle);
});
