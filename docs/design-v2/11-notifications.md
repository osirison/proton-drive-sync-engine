# Notifications

Linux desktop notifications via libnotify / the XDG portal. GNOME shows them **top-centre**; KDE
bottom-right. There are no designed notifications in the current build.

## The design is mostly a list of what stays silent

A sync app knows about hundreds of events a day. Tell someone about all of them and they switch
notifications off — and then the one that mattered (*a folder is about to be deleted from your
computer*) never arrives.

The test: **would this person want to stop what they're doing?**

## Four events interrupt

| Event | Why | Icon state |
| --- | --- | --- |
| **Something would be deleted permanently** | The one event where silence can cost files you'd never get back | struck/filled `#FF3B3B` |
| **A file changed on both sides** | Nothing is lost, but you're now editing two versions without knowing | needs-you `#FF6B6B` |
| **The first sync finished** | Once, at the end of a long wait you were told to walk away from | settled |
| **Nothing has synced for a day** | Not a blip — an outage, expired session, or full disk | struck `#FF3B3B` |

## Twelve categories deliberately stay silent

every sync pass · every file sent · every file received · folders followed · renames ·
a single failed pass · retries · scheduled sweeps · skipped files · pause and resume ·
recoverable deletions · settings saved

All of it is in Activity, where you go looking for it.

## The hard rule

**Nothing destructive is ever a notification button.** Delete, discard a version, approve all — none
of them appear in a banner. A banner may offer `Keep them`, because keeping is reversible and
safe. Irreversible things happen in a window where you can see what you're losing, not in a corner
of the screen you might swat at.

## Banner spec

`width:372px` (or the desktop's own width), `background:rgba(18,20,25,.94)`,
`border:1px solid rgba(255,255,255,.11)`, `border-radius:16px`, `padding:15px 17px`,
shadow `0 18px 44px rgba(0,0,0,.5)`. Use the desktop's own notification chrome where the platform
provides it — these values are for when it doesn't.

Layout: `display:flex; gap:13px` — a `34px` hexagon in the matching state (so the banner and the
tray agree), then a column:
1. Header row: `Drive Sync` 12px/600 `#99A2AE`, `flex:1` spacer, relative time in mono 11px
   `#6D7783` (`now`, `2m ago`, `14:12`).
2. Title 13.5px/600 `#F2F4F7`, `margin-top:5px`.
3. Body 12.5px `#C9D0DA` (or `#99A2AE` when nothing is wrong) `line-height:1.5`,
   `margin-top:4px`.

Actions row `margin-top:12px; gap:8px`, both `flex:1`, `padding:8px; border-radius:9px;
font-size:12.5px`. Primary: `border:1px solid rgba(255,255,255,.14);
background:rgba(255,255,255,.06); color:#F2F4F7; 600`. Secondary:
`border:1px solid rgba(255,255,255,.1); background:transparent; color:#C9D0DA`.

## The four, verbatim

**Permanent deletion** — icon `#FF3B3B` outline + filled centre.
Title: `1,204 photos would be deleted from this computer`
Body: `**photos/2019** was deleted on Proton Drive. Nothing has happened here yet.` (path in mono 12px)
Actions: `Keep them` / `Review` — note the safe action is primary and there is **no Delete**.

**Conflict** — icon `#FF6B6B`.
Title: `You both changed notes/todo.txt`
Body: `Both versions are safe. Pick one when you have a moment.`
Actions: `Compare` / `Later`

**First sync done** — icon settled, stroke `#6D7783`, check `#E8EBF0`.
Title: `Both sides now match`
Body: `First sync finished — 12,480 files, 41.2 GB. Nothing was deleted.`
No actions.

**Outage** — icon struck `#FF3B3B`.
Title: `Nothing has synced since yesterday`
Body: `Proton Drive is asking you to sign in again. 61 changes are waiting — nothing is lost.`
Actions: `Sign in` / `Open Drive Sync`
**"nothing is lost" comes before the problem.**

## Grouping

Several conflicts do not mean several banners. One banner, with the count as the hexagon's numeral
(mono, `font-size:44` at `34px` render, `y=74`):
Title `5 files changed on both sides` / Body
`All ten versions are safe. This usually means another device was offline for a while.` /
Actions `Go through them` / `Later`.

Coalesce within a 30-second window. Never stack more than one Drive Sync banner.

## At two folders or more

Everything above is a banner about one folder. When the daemon runs two or more
[folder pairs](../adr/0005-multiple-folder-pairs.md), a banner is about **one of them**, and says which:

- **The header row names the folder.** `Drive Sync · photos` in place of `Drive Sync` — the same 12px/600
  `#99A2AE` span, one node, a middle dot and the folder's name. No sentence of the four changes and none
  grows: the deletion banner's body still opens with its mono path. A desktop's notification server draws
  that line as its own header (it is the notification's application name), so the folder is named without
  touching the title or the body. With **one** folder the line is `Drive Sync` and the banner is the banner
  above, byte for byte — "two or more" is the daemon's count of pairs, not the number of folders that happen
  to have something to say. (Drawn: `11a Two folders`.)
- **Each folder has its own memory; the banner has one.** What was said, whether the first sync was
  witnessed, and the newest sync seen are kept per folder, so the same queue in two folders is two things to
  say, adding a folder does not make the first one repeat what it has said, and a folder added later
  announces **its own** first sync, once. The 30-second window and "a deletion may interrupt a quieter banner"
  are about the banner on screen, whatever folder it is about: two conflicts in two folders are one banner now
  and the second later, never two at once. Folders are considered in the order the daemon lists them, most
  serious event first.
- **A pause is the folder's.** A folder the person paused does not trigger the outage banner, however long
  ago it last synced; another folder, not paused, does.
- **Only what is known is said.** A folder is read from what the daemon last said about it and nothing older:
  while the daemon is not answering, no banner is raised about any folder but the one on screen (read, as at
  one folder, from what that folder last showed), and what was said about the others is not forgotten.
- **The buttons act on the folder the banner named**, not the one the window happens to be showing when it is
  pressed — which can be minutes later and a different folder. `Keep them` keeps that folder's permanent
  deletions and no other's. `Review` and `Compare` switch the window to the folder **first** and open the
  screen only then (if that folder is no longer there, the window opens and goes nowhere). `Try again now`
  syncs that folder alone. `Open Drive Sync` opens the window on that folder.

## Settings — three choices, not twelve switches

`When to interrupt me` 18px/600 / `Everything else stays in Activity regardless.`
Same radio-card pattern as the deletions tab:

1. **Only when you need me** — `default` badge.
   `The four events on the left. Roughly once a week, in a quiet month.`
2. **Only permanent deletions** —
   `The single event that can cost you files. Conflicts wait quietly in the app.`
3. **Never** —
   `The menu bar glyph still changes, and things still wait for you rather than happening on their own.`

Key: `notify_policy`. **"Never" must not change engine behaviour** — deletions still wait for
approval. Turning off notifications is not consent.
