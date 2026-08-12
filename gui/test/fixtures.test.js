// The two things about the F9 fixtures that a reader cannot check by reading them (#173).
//
// `check-fixtures.mjs` already gates the registry — every in-scope frame present, of the shape its
// class implies, no dead `fids` slot, no fixture reading the wall clock. What it deliberately does
// NOT do is re-derive the daemon's own arithmetic, and that is the one thing here that was wrong
// once and would be wrong silently again.
//
// A `DryRunReport` is `{ summary, plan }` parsed verbatim from the daemon's stdout, and the daemon
// builds the summary from the plan: `PlanSummary::from_plan` sets `total: plan.len()` and increments
// exactly one counter per row (src/sync.rs). So a fixture whose summary and plan disagree describes
// a reply the thing it imitates cannot produce — and the screen written against it looks right.
//
// `9a Review` shipped exactly that: `total: 471` beside `plan: []` and counters summing to 474, from
// a comment that added three of the four contributors. `summaryOf` makes it unrepresentable and this
// test makes sure nobody reintroduces a hand-written summary later.

import { test } from "node:test";
import assert from "node:assert/strict";
import { FIXTURES, resolveFixture } from "../src/js/fixtures/frames.js";
import { action, bulk, summaryOf } from "../src/js/fixtures/dryrun.js";
import { EMPTY_CONFIG } from "../src/js/api.js";

/** Every counter except the total and the one derived from three others. */
const CONTRIBUTORS = (summary) =>
  Object.entries(summary)
    .filter(([key]) => key !== "total" && key !== "destructive_actions")
    .reduce((sum, [, value]) => sum + value, 0);

test("every fixture's dry-run summary is one the daemon could have emitted", () => {
  const seen = [];
  for (const label of Object.keys(FIXTURES)) {
    const report = resolveFixture(label)?.dryRun?.report;
    if (!report) continue;
    seen.push(label);
    assert.equal(report.summary.total, report.plan.length, `${label}: total must be plan.len()`);
    assert.equal(
      CONTRIBUTORS(report.summary),
      report.summary.total,
      `${label}: the per-action counters must sum to the total`,
    );
  }
  // If this ever reads zero the assertions above are vacuous, which is the way a test like this dies.
  assert.ok(seen.length >= 3, `expected the 5a and 9a frames to carry a dry run, saw ${seen.length}`);
});

test("destructive_actions is the display set, which is not the gated set", () => {
  // `plan.rs` encodes the distinction the design conflated: a `purge` is tinted and sorted first but
  // must NEVER force the typed-DELETE gate, because it destroys no user data. A fixture that let the
  // two drift apart would let S4 be written against the wrong one.
  for (const label of Object.keys(FIXTURES)) {
    const payload = resolveFixture(label)?.dryRun;
    if (!payload) continue;
    const { summary, plan } = payload.report;
    assert.equal(
      summary.destructive_actions,
      summary.remote_deletes + summary.local_deletes + summary.purges,
      `${label}: destructive_actions sums the three display-destructive counters`,
    );
    const gated = plan.some((row) => row.action === "remote_delete" || row.action === "local_delete");
    assert.equal(payload.requires_delete_gate, gated, `${label}: the gate keys on delete_direction()`);
    assert.deepEqual(
      payload.files_at_risk,
      plan.filter((r) => r.action === "remote_delete" || r.action === "local_delete").map((r) => r.path),
      `${label}: files_at_risk is exactly the gated rows' paths`,
    );
  }
});

test("summaryOf counts a purge as display-destructive but never as a gated action", () => {
  const summary = summaryOf([
    action("a.txt", "upload"),
    action("gone.txt", "remote_delete"),
    action("stale-record", "purge"),
  ]);
  assert.equal(summary.total, 3);
  assert.equal(summary.uploads, 1);
  assert.equal(summary.remote_deletes, 1);
  assert.equal(summary.purges, 1);
  // Both the delete and the purge; `requires_delete_gate` would see only the delete.
  assert.equal(summary.destructive_actions, 2);
});

test("summaryOf throws on an action name the daemon does not have", () => {
  // Silently ignoring one would produce a summary whose parts do not sum to its total — the exact
  // defect this module exists to prevent — so a typo must be loud.
  assert.throws(() => summaryOf([action("x", "delete_everything")]), /no PlanSummary counter/);
});

test("every fixture's wire status is a word the daemon actually sends", () => {
  // `ControlResponse.status` is the DAEMON's own word and it is only ever these three
  // (src/daemon.rs). `idle` is the derived `DaemonState` and belongs on the payload's `state`. Three
  // fixtures put `idle` on the wire — inert today, because `derive_state` never reads the string,
  // and a trap for the first screen or test that does.
  const SENT = new Set(["running", "paused", "syncing"]);
  for (const label of Object.keys(FIXTURES)) {
    const status = resolveFixture(label)?.status?.response?.status;
    if (status === undefined) continue;
    assert.ok(SENT.has(status), `${label}: response.status "${status}" is not one the daemon sends`);
  }
});

test("a broken sameAs chain resolves to null rather than a half-built fixture", () => {
  // Fail closed. Returning the partially-resolved entry gives a frame with a dangling `sameAs` and
  // none of its twin's data — quietly wrong rather than absent — and makes check-fixtures.mjs's
  // "chain does not resolve" branch unreachable while it claims to check exactly this.
  const cyclic = { A: { sameAs: "B" }, B: { sameAs: "A" } };
  const missing = { A: { sameAs: "nope" } };
  // Re-implemented against a local table, because `resolveFixture` closes over the real registry —
  // which is the point of the assertion below: the real one has no broken chain to test with.
  const resolve = (table, label, seen = new Set()) => {
    const entry = table[label];
    if (!entry) return null;
    if (!entry.sameAs) return entry;
    if (seen.has(label)) return null;
    seen.add(label);
    const twin = resolve(table, entry.sameAs, seen);
    if (!twin) return null;
    const { sameAs: _drop, ...own } = entry;
    return { ...twin, ...own, fids: entry.fids };
  };
  assert.equal(resolve(cyclic, "A"), null, "a cycle must not resolve");
  assert.equal(resolve(missing, "A"), null, "a missing twin must not resolve");

  // And the real registry has none: every `sameAs` resolves to something.
  for (const label of Object.keys(FIXTURES)) {
    assert.notEqual(resolveFixture(label), null, `${label}: does not resolve`);
  }
});

test("the mock's no-config reply has the same shape as a real one", () => {
  // `read_config` cannot fail to send a full `ConfigPayload`: an absent file loads as an empty doc,
  // not an error, so every field is still filled in. The mock returned `{}` for a frame describing
  // no config file, which left `config.exclude` undefined — a `.map` over it throws in browser
  // preview and nowhere else, which is the worst place for a difference to live.
  //
  // Pinned against a fixture that carries a REAL config rather than a hand-copied key list, so the
  // day `ConfigPayload` grows a field, whichever of the two is updated first drags the other along.
  const real = resolveFixture("8a Settings").config;
  assert.deepEqual(
    Object.keys(EMPTY_CONFIG).sort(),
    Object.keys(real).sort(),
    "the empty reply must carry every field a populated one does",
  );
  for (const key of ["include", "exclude"]) {
    assert.ok(Array.isArray(EMPTY_CONFIG[key]), `${key} is a Vec<String> on the wire — never null`);
  }
  assert.equal(EMPTY_CONFIG.exists, false, "a missing file reports exists:false, not a missing reply");
});

test("bulk generates deterministic rows, because a fixture may not vary between runs", () => {
  assert.deepEqual(bulk("p", "upload", 3), bulk("p", "upload", 3));
  assert.equal(bulk("p", "upload", 3).length, 3);
  assert.equal(bulk("p", "upload", 3)[0].path, "p/0000");
});
