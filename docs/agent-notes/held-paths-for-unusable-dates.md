---
trigger: a change to HeldPaths, paths_to_hold, plan_sync_entities_with_stats_held, compute_directory_deletion_verdicts, UnusableDate or UnsyncableReason::LocalUnusableDate; a file or folder dated before 1970 (unzipped archive, 1881); local_unusable_date in the unsyncable list or the plan report; a synced item that "plans a delete" after its date went bad
depends_on: src/daemon.rs (HeldPaths, paths_to_hold, build_plan_report, execute_plan_and_commit), src/sync.rs (plan_sync_entities_with_stats_held, compute_directory_deletion_verdicts), src/index.rs (UnusableDate, modification_seconds, scan_local_tree)
recorded: 2026-11-17
---

# A held path is cut from all three maps, and its parent must not be deleted around it

**What a hold is.** A file or folder whose modification date the engine cannot use (before 1970, or
unreadable) is skipped on its own by the local walk through the typed `UnusableDate` error. It never
fails the walk. A held folder is held whole: not descended into, reported alone. The index stores signed
seconds; it is `SystemTime::duration_since(UNIX_EPOCH)` that refuses such a date.

**Why a hold must cut the local entities, the remote map AND the base index** (`HeldPaths::without`).
A path merely missing from the local scan reads as locally deleted: a synced file plans a `RemoteDelete`,
a folder a recursive one, and a remote copy with no index row a download over the local file. The cut
is the same one an exclude rule makes (the selective-sync invariant). Both planner callers apply it,
`build_plan_report` and `execute_plan_and_commit`; if only one did, every `apply` would re-plan to a
different token than the one reviewed and be refused.

**Release rule.** Only a **full** walk drops a held item (it plans against the true remote state, and
absence ends the hold). An incremental pass plans around its own walk's holds plus every standing held
item, and when a standing one is no longer seen bad it latches `force_full_walk` instead of releasing
it, because the event cursor kept moving while the item was held and a remote edit may lie behind it.
Releasing early would upload over that edit.

**Parent-delete rule (the hole found in review of PR 521).** Cutting the held path out of the maps also
cuts it out of the planner's proof that a folder's subtree is clean. With the remote folder deleted,
`compute_directory_deletion_verdicts` would prove the parent clean and plan a recursive `LocalDelete`;
the executor then disposed of the whole folder with the held item inside (trash by default, gone for
good with `local_delete_mode = permanent`). So the planner is told the held paths
(`plan_sync_entities_with_stats_held`, a `held` set; the old entry point passes an empty one) and each
held path falsifies its nearest tracked ancestor's verdict, exactly as an untracked live entity does.
The parent then falls back to the planner's own non-destructive answer (recreate the folder on Proton;
only the files that are not held are deleted locally). The same loop covers the other direction (a
folder deleted locally while a held child's remote copy is cut from the map: no recursive
`RemoteDelete`). Any new planner caller must pass the same held set it cut by.

**Residual: a rename.** Renaming a held file (`old.txt`, with an index row) to `new.txt` keeps its date,
so `new.txt` is held and `old.txt` is simply gone, which plans a `RemoteDelete old.txt` (it waits for
delete approval under the default policy). A plain local delete reads the same way.
