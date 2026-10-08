---
trigger: CARGO_TARGET_DIR, git worktree, git archive, building an older commit beside the current one, "no X in config" for a function that exists, cargo uses a stale workspace crate
depends_on: Cargo.toml (workspace), gui/gui-core, gui/src-tauri
recorded: 2026-10-08
---

# Two checkouts of this workspace must not share a `CARGO_TARGET_DIR`

**Symptom:** after building an older commit (a golden capture, a baseline, a bisect step) in a second
directory with `CARGO_TARGET_DIR` pointed at this repo's `target/`, the current tree stops compiling
against its own code:

```
error[E0432]: unresolved imports `proton_drive_sync_engine::config::PairView`, `...::pair_views`
  |  no `pair_views` in `config`
```

The function exists, in the file, in the current tree. `cargo check` of the same crate can pass at
the same moment, because `check` writes different artifacts from `build`/`test`.

**Cause (measured):** cargo's unit hash for a path crate does not include where the workspace sits.
Two checkouts with the same layout and the same dependency versions therefore produce the **same
artifact file names** (`libproton_drive_sync_engine-<hash>.rlib`, and the same
`proton_sync_gui_lib-<hash>` test binary — the hash printed by `Executable unittests` was identical
in both trees). Freshness is then decided by source mtimes against the last build's dep-info. The
second tree's build overwrote the rlib; the first tree's sources were *older* than that build, so
cargo called them fresh and linked the other tree's engine. (The identical hash and the error were
measured; the mtime step is inferred from the fix working — the fingerprint files were not read.)

**Fix, now:** touch the sources of the tree you want rebuilt (`touch` every `.rs` and `Cargo.toml`
under `src/`, `gui/gui-core/` and `gui/src-tauri/`), then build. The recovery is one rebuild.

**Fix, next time:** give the second checkout its own `CARGO_TARGET_DIR` (a scratch directory — it
costs a full dependency build, a few minutes) and delete it afterwards. `/tmp` here is tmpfs with a
few GB free; a debug build of this workspace is far larger, so use the disk, not `/tmp`.

**It is silent in the dangerous direction.** Nothing says a stale crate was linked; the failure
above shows only because a new public item was missing. A *behaviour* change in the older tree would
make the current tree's tests run old code and pass or fail for reasons that are not in the diff. If
a test result is surprising after an older tree was built, touch and rebuild before believing it.

**Capturing what an old commit sends, safely:** `git archive <sha> | tar -x -C <scratch>` (no
`.git` metadata is touched, unlike `git worktree add`), copy in only the test harness, and run the
one test with `XDG_CONFIG_HOME` and `XDG_RUNTIME_DIR` pointed at scratch directories so
`RuntimePaths::resolve()` in the old code reads nothing real. The harness itself can assert the
variable names the scratch directory and refuse to run otherwise.
