# Copy deck

Every user-visible string, verbatim. Recreate exactly — the copy is load-bearing, especially
around deletion.

## Voice rules

1. **Say what happens to files, not what the daemon does.** "Notice changes the moment they happen",
   not "Event-driven reconcile".
2. **Consequences in things you'd miss.** "1,204 photos, 8.4 GB", not "directory, recursive".
3. **Reassurance before the problem.** "Nothing is lost. 4 changes are waiting…" then the failure.
4. **Never paraphrase a daemon error.** Show the exact string in mono.
5. **No exclamation marks, no emoji, no "Oops".** No sentence-case shouting: "Deleting" not "DELETING"
   — except the typed word `DELETE` itself.
6. **"this computer"**, never a brand or OS name. **"Proton Drive"** in full, never "the cloud",
   never "remote".
7. Say **"kept"** not "preserved", **"waiting"** not "pending", **"brought here"** not "downloaded"
   in prose (the mono/technical tier may say uploaded/downloaded).

## Product and chrome

`Proton Drive Sync` · `~/ProtonDrive ⇄ /Drive/RemoteFolder`
Status chips: `idle` · `syncing` · `3 waiting` · `2 waiting` · `rehearsal · nothing has changed` · `step 1 of 2`
Footer doors: `Activity` · `Plan a sync` · `Settings` · `Details`

## Main screen

- `Everything is up to date` / `last synced 2 minutes ago · 12,480 files · 41.2 GB`
- `Syncing 3 changes` / `started 14 seconds ago · 2 leaving, 1 arriving` / `3 other changes are waiting on you`
- `Paused` / `7 changes have piled up since 13:20. Nothing will move until you resume.`
- Side labels: `This computer` · `Proton Drive` (compact: `This computer` · `Proton`)
- Buttons: `Sync now` · `Pause` · `Resume syncing` · `queued`
- Footer lines: `~/ProtonDrive ⇄ /Drive/RemoteFolder` · `386 MB sent · 1.1 GB received today`
- Attention band:
  `One file changed on both sides` / `notes/todo.txt · both copies kept, nothing lost` / `Compare`
  `Two deletions are waiting on you` / `1 removes from this computer permanently · 1 goes to Proton's Trash` / `Review`
  - Third clause, **drawn by no frame**: a local deletion in trash mode reads `1 goes to this computer's Trash`, between the permanent clause and Proton's. A clause with a count of zero is dropped, never printed — so the drawn sentence above is the queue that has one of each of the other two.
- Compact: `Up to date` / `2 minutes ago` / `12,480 files` · `3 things need you` /
  `One file changed on both sides.` `Two deletions are waiting.` / `Review them` /
  `syncing continues` / `Later` / `Open`

## Conflicts

- `You both changed this file` / `Nothing has been lost — both versions are still here, and syncing carries on around this.`
- `1 of 3` · `a plain text file · last agreed 3 hours ago`
- `Your version · this computer` / `You added a line, 5 minutes ago` /
  `Yours has buy milk where Proton's has something else, and is otherwise the same.`
- `Proton's version · from another device` / `Changed a line and added one, 2 minutes ago` /
  `Proton's has buy oat milk and an extra line at the end.`
- `See the exact differences` / `Hide differences` / `Open both in an editor`
- `Keep mine` / `Your version goes to Proton Drive. Proton's version is discarded.`
- `Keep both` / `Nothing is lost. Proton's copy lands beside yours as todo.proton-cloud.txt.`
- `Use Proton's` / `Proton's version replaces the file on this computer. Yours is discarded.`
- `Discarding a version can't be undone from here.` / `Decide later`
- `Two lines differ. Everything else in the file matches.` / `2 lines differ · 3 lines identical` / `not in your version`
- `Still waiting after this one` / `both changed it` / `a folder here, a file there`
- Cleared: `Nothing left to decide` / `You settled 3 files. Two kept both versions, one took Proton's copy.` / `Back to sync`

## Deletions

- `Two files are waiting to be deleted` / `They were deleted on one side. Nothing happens to the other side until you say so — syncing carries on around them.`
- `Permanent · this computer` / `Removed straight from disk. Not moved to any trash, and not recoverable from Proton.`
- `Recoverable · Proton Drive` / `Moved to Proton Drive's Trash. You can restore it there until the trash is emptied.`
- `Deleting this removes 1,204 photos, 8.4 GB from this computer, including everything inside it.`
- `You deleted this on this computer. Deleting it on Proton moves it to Proton Drive's Trash, where you can still get it back.`
- `deleted on Proton 22m ago` · `last opened Mar 2024` · `deleted here 6m ago` · `last edited Jan 2026`
- `To delete it, type DELETE below.` / `Delete` / `Move to Proton's Trash`
- `Keep it — put it back on Proton Drive` / `Keep it — bring it back to this computer`
- `Deletions stay here until you decide. Nothing expires.` / `Keep both files`
- Armed: `Delete 1,204 photos from this computer?` /
  `Everything in photos/2019 — 8.4 GB — is removed from disk. It does not go to your trash, and it is already gone from Proton Drive, so there is nothing to restore it from.`
  / `Delete permanently` / `Press Esc to cancel.`
- Empty: `Nothing waiting to be deleted` / `When a file disappears from one side, it waits here for you instead of vanishing from the other.`
- Compact: `2 files waiting to be deleted` / `1,204 photos gone from this computer, permanently` / `to Proton's Trash — recoverable` / `Review them`
- Trash mode — **no frame draws any of these** (DEVIATIONS §100). A local deletion goes to this computer's Trash by default, so the local card moves into the recoverable column and the typed-`DELETE` gate does not arm for it. Everything above stays exactly as drawn and is what the screen says once permanent deletion is turned back on.
  - `Recoverable · this computer` / `Moved to this computer's Trash. You can restore it from your file manager.`
  - Both kinds under one header: `Recoverable` / `Each of these can be brought back — from Proton Drive's Trash, or from this computer's.`
  - `You deleted this on Proton Drive. Deleting it here moves it to this computer's Trash, where you can still get it back.`
  - `Move to this computer's Trash`

## Plan a sync

- `The next sync moves 9 things` / `One of them can't be undone. Everything here is a rehearsal — nothing has changed yet.` / `Check again`
- `Leaving this computer` / `files, 4.1 MB` / `Plus one new folder created on Proton Drive to hold them.`
- `Arriving from Proton` / `files, 2.6 MB` / `One file you renamed will be renamed here to match.`
- `One file gets deleted for good` / `archive/old-notes.md is removed from Proton Drive. It's already gone from this computer, so nothing will bring it back.` / `Leave it alone`
- `Every action, in order` / `9 actions · 1 conflict kept as both copies`
- Outcomes: `deleted for good on Proton` · `sent to Proton` · `brought to this computer` ·
  `folder created on Proton` · `moved to match Proton` · `both copies kept, nothing lost`
- `type DELETE to allow it` / `Only needed because this plan deletes something.` /
  `Run it without the deletion` / `Run this sync`
- Safe: `Nothing gets deleted` / `Five files move, both sides end up with everything. This plan is safe to run.` / `new folder` / `moved` / `Checked 40 seconds ago against both sides.`
- Checking: `Working out what would change` / `Comparing both sides. Nothing is being touched.` / `8,431 of 12,480 files` / `Stop`

## Activity

- `Activity` / `Nothing has needed to move since 14:32. Both sides matched at the last check, 2 minutes ago.`
- `Check a file — type any name or path` / `Ctrl F` / `1 match`
- `Both sides agree` / `watched continuously · checked 2m ago` / `next full check in 4m`
- `4 files are never synced` / `They sit in your folder but aren't copied anywhere. Two match a rule you wrote; two can't be synced at all.` / `Show them`
- `Last things to move` / `7 files in the last 3 days` / `Quiet is normal — most days nothing needs to move.` / `All 7 files` / `Sync passes`
- Row outcomes: `sent to Proton` · `brought here` · `both copies kept` · `moved to match` · `skipped, can't be synced`
- Lookup: `Safely on both sides` / `Identical here and on Proton Drive since 14:32 today.` /
  `This file's history` / `Sent to Proton Drive` / `Both sides had changed — you kept yours` /
  `First brought to this computer` / `Open folder` / `Open on Proton Drive` / `linked · id 4c8f…9a21`
- Pending: `On its way to Proton Drive` / `Started 8 seconds ago · 2.8 MB` / `only on this computer so far`
- Never-synced dialog: `4 files are never synced` / `They live in your folder but no copy exists on Proton Drive.` /
  `You told it to skip these` / `A rule in your settings matches them: *.tmp` / `Change this rule` /
  `Can't be synced` / `Not real files — Proton Drive has nothing to store for them.` /
  `a socket` / `a shortcut` / `Nothing here is at risk — it's just not backed up.` / `Done`
- Passes: `18 of the last 20 passes finished cleanly. One failed and retried on its own.` /
  `Last 20 passes` / `how long each took · 12:45 onward` / `most recent 14:32` /
  `Finished cleanly` / `Couldn't reach Proton Drive` / `retried at 14:17 and worked` /
  `proton-drive: connection timed out after 60s` / `nothing to do` /
  `Only the last 20 passes are kept. Anything older lives in the system log.` / `Open the system log`
- Quiet: `Nothing has moved in the last hour.`
- Details: `Copy all`

## Settings

- `Settings` / `Changes here take effect on the next sync. Nothing is written until you save.`
- Tabs: `Folders` · `What to skip` · `Deletions` · `Advanced`
- `The pair being kept in step` / `Choose…` / `12,480 files, 41.2 GB in here today. Changing it starts a fresh merge — nothing gets deleted.` / `Folder on your Proton Drive. Must already exist.`
- `How often it checks` / `Notice changes the moment they happen` / `Proton tells the app when something changes on another device, so it syncs within seconds.`
- `Compare everything, top to bottom` / `A full check of all 12,480 files as a safety net. It's slow, so it runs on a schedule rather than constantly.` / `Weekly` / `Monthly` / `Every` / `at` / `On day` / `Months without a 15th are skipped to the last day.`
- `Run one now` / `Full sweep now` / `Takes about 4 minutes; syncing keeps working. Last one 2 days ago — nothing was out of step.` / `Sweep now`
- `Anything matching a rule below stays on this computer and is never copied to Proton Drive. Rules are matched against the path inside your sync folder.`
- `Your rules` / `hiding 4 files, 3.1 GB in total` / `Skipping 2 files right now` / `Skipping 2 files, 3.1 GB` / `added 14 Jul · the folder still exists on this computer` / `Matching nothing` / `no such folder here any more — safe to remove` / `Remove` / `Add a rule — e.g. *.psd or scratch/**` / `Add`
- `Two more files can't be synced no matter what — a socket and a shortcut. Nothing you can change here.` / `See them` / `The app's own .sync folder is always skipped and can't be added here.`
- `When a file is deleted` / `Deleting on one side would normally delete it on the other. This is how much say you get.`
- `Ask me every time` / `recommended` / `Deletions wait in a queue until you approve them. Nothing disappears behind your back.`
- `Only ask about permanent ones` / `Deletions that go to Proton's Trash happen automatically. Anything removed from this computer for good still waits for you.`
- `Never ask` / `Deleting a file on either side deletes it on the other immediately, including permanently from this computer.`
- Disposal panel — **no frame draws it** (DEVIATIONS §100a). A second panel on the same Deletions tab, beneath the one above: that one decides whether a deletion waits for you, this one decides what a local deletion does when it goes ahead.
  - `What deleting does to your copy` / `This is about files on this computer. Anything deleted on Proton Drive always goes to Proton's Trash.`
  - `Move them to the trash` / `Deleted files go to this computer's Trash, where you can restore them from your file manager. They keep taking up space until you empty it.`
  - `Delete them permanently` / `Deleted files are removed from the disk straight away, freeing the space. There is no trash to get them back from.`
- `Saving writes only what you changed. Your comments and anything the app doesn't understand are left alone.` / `Discard changes` / `Save` / `One rule removed — 2 files, 3.1 GB will start syncing.`
- Refused: `That folder doesn't exist on Proton Drive` / `Nothing was saved — your old settings are still running. Create the folder on Proton Drive first, or pick a different one.` / `remote_root: /Drive/Archive2026 — not found` / `Go back and fix it` / `Create it on Proton Drive`

## Onboarding

- `Which two folders should match?` / `One on this computer, one on Proton Drive. From then on they stay identical.`
- `Choose a different folder…` / `Browse Proton Drive…` / `A new empty folder is fine — everything on Proton Drive will be brought down into it.` / `Signed in as you@proton.me · 39.1 GB of 500 GB used`
- `You can tell it to skip things — screenshots, huge exports, scratch folders — now or any time later in Settings.` / `Add skip rules`
- `Nothing is copied or changed until you approve the plan.` / `See what will happen`
- `Nothing gets deleted today` / `The first sync only adds. Files you have go up, files on Proton come down, and anything that exists on both sides in different versions is kept as two copies so you can look at them later.`
- `Going up to Proton` / `Files that only exist on this computer.` / `Coming down to this computer` / `Needs 38.4 GB free. You have 214 GB.`
- `11,798 files already match on both sides` / `left alone`
- `2 files differ on both sides` / `both copies kept — you decide later`
- `3 files can't be synced — a socket and two shortcuts` / `skipped`
- `Nothing will be deleted` / `on either side`
- `worked out 40 seconds ago · about 25 minutes to finish` / `See all 471 actions` / `Back` / `Start the first sync`
- `Bringing everything together` / `159 of 471 done · about 17 minutes left` / `44 sent` / `115 received` / `You can close this window — it keeps going in the background.` / `nothing deleted · 2 conflicts kept as copies`
- `Both sides now match` / `12,480 files, 41.2 GB. Nothing was deleted, and 2 files are waiting for you to pick a version.`
- `One thing to agree to before it runs on its own` / `From now on, deleting a file on either side deletes it on the other. You'll be asked before each one — and you can change that in Settings — but this is how the two folders stay identical.` / `I understand deletions travel both ways.` / `Syncing stays paused until you agree.` / `Start syncing`
- `Proton Drive's command line tool isn't installed` / `This app drives the official tool rather than talking to Proton directly. Install it once and setup will carry on. Detected Debian — other distributions are in the help.` / `sudo apt install proton-drive` / `Copy` / `Check again` / `Installation help`

## Tray

`Open Drive Sync` · `Sync now` · `Pause syncing` · `Resume syncing` · `Try again now` ·
`Close window` + `keeps syncing` · `Quit` + `stops syncing`
`Can't reach Proton Drive` / `Nothing is lost. 4 changes are waiting and will go as soon as it's back.` / `retrying in 40s · last reached 13:58`

## Notifications

See `11-notifications.md` — all four are quoted verbatim there.
