---
trigger: gui/tools/fidelity/check-pair-routing.mjs, fidelity:pairs, a new Tauri command, "Cannot access 'X' before initialization", a scenario helper, Bridge.answer
depends_on: gui/tools/fidelity/check-pair-routing.mjs
recorded: 2026-10-11
---

# `fidelity:pairs`: a helper below its first use, and a command the stand-in never heard of

Two ways the page gate goes quiet or breaks when a scenario is added for a new command (found adding the
Proton-folder confirmation, #453).

## 1. A `const` helper is in its temporal dead zone for every scenario above it

**Symptom:** a scenario fails with `Cannot access 'dialogText' before initialization` although the helper is
in the file.

**Cause:** the file is one module with top-level `await scenario(...)` calls that run **in file order**.
`const dialogText = …` declared 3,000 lines down does not exist yet when the scenario at line 2,200 runs.
A `function` declaration would hoist; every helper here is a `const` arrow.

**Fix:** declare a helper above the first scenario that uses it (next to `pageText`, `press`, `select`),
not beside the scenarios it was written for.

## 2. `Bridge.answer` returns `null` for a command it has no case for

**Symptom:** none. A new command's scenario, or an old scenario that should now pass through the new
command, goes green without ever reaching it.

**Cause:** `default: return null`. For a command whose `null` reads as "no" (the Proton-folder question,
`check_proton_folder_change`: `answer?.changed` is falsy), every existing scenario keeps taking the old path.

**Fix:** give the Bridge a case for the command, as Rust answers it, **before** relying on a scenario; then
rerun the whole gate, because the scenarios that used to take the old path (here the two that type a Proton
folder and expect `write_config`) now fail until they follow the new one. Poison the page code
(`return null` from the check) and see a scenario fail, or the scenario proves nothing.
