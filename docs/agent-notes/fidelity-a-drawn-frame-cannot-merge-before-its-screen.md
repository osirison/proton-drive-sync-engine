---
trigger: npm run fidelity after npm run fidelity:extract, "unclaimed drawn node(s)", KNOWN_UNCLAIMED, adding a frame to Drive Sync.dc.html, a design-only PR, "screen not built"
depends_on: gui/tools/fidelity/assert.mjs, gui/tools/fidelity/known-deviations.mjs (classifyUnclaimed), .github/workflows/ci.yml (fidelity job)
recorded: 2026-10-09
---

# A frame drawn in the prototype fails `npm run fidelity` until the app builds it

**Symptom:** a PR that adds frames to `docs/design-v2/Drive Sync.dc.html`, re-extracts them and builds
nothing else is expected to pass with the new frames listed as "screen not built". It fails instead:

```
fidelity:assert: N unclaimed drawn node(s). Declare the slot, or add a KNOWN_UNCLAIMED entry.
```

**Cause (read in `assert.mjs`, not run):** the "screen not built" list is only for a frame that has no
`fids` map. The unclaimed-node census runs for every frame **before** that bail-out. Any node with a
non-empty key that no slot claims and no `KNOWN_UNCLAIMED` row names is reported, and the run exits 1
on it. `KNOWN_UNCLAIMED` rows carry exact keys, never a prefix. A new frame has from about forty to
several hundred keyed nodes (`9a-folders.json` has 43). The `fidelity` job in CI is not filtered by path.

**Rule:** draw a frame in the PR that builds it, with its fixture and `fids` map. Phase 5 split its PRs
this way (ADR 0005, "Phase 5 shipped, with departures") for this reason. A frame that cannot be built
yet needs one `KNOWN_UNCLAIMED` row per node, each with a class and, for `issue`, an open number.
