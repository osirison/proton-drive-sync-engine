# Onboarding — first run

**Purpose:** get two folders chosen, make the first merge feel safe, and get informed consent for
two-way deletion.

## What changed and why

The old wizard had four steps: **CLI check → Folders → Review plan → Start service**. Step 4 existed
for the daemon, not the person. Step 1 was a dependency audit shown before anything the user cares
about. Step 3 buried the important reassurance under five counters and put a two-way-deletion
acknowledgement checkbox *before* the merge had even run.

Now: **two steps.** The CLI check is a silent precondition that only surfaces if it fails. The
review step makes one promise. The consent moves to after the merge, where it's actually true.

Onboarding is also where the seam gets **assembled** — you pick a side, then the other, then watch
them join. By the time the app is running you already know what the two halves of every later
screen mean, with no tour.

Header shows `step 1 of 2` / `step 2 of 2` in mono 11px `#6D7783` instead of a status chip.

## Step 1 — Which two folders?

Centred title block `padding:20px 32px 0`:
`Which two folders should match?` 30px/600/`-0.03em` /
`One on this computer, one on Proton Drive. From then on they stay identical.` 14px `#828B98`.

Seam `top:0; height:250px`, grid `1fr 1fr` with `padding-right/left:36px`:

| | Left | Right |
| --- | --- | --- |
| Label | `8px` dot `#FF9F1C` + `This computer` | `Proton Drive` + dot `#22D3EE`, right-aligned |
| Card | `border:1px solid #2E323A; border-radius:14px; background:#101216; padding:18px 20px` | same, contents right-aligned |
| Path | mono 14px `#F2F4F7`, `word-break:break-all` | same |
| Stats | `margin-top:12px; padding-top:12px; border-top:1px solid #1A1D22` — `341 files` / `2.1 GB` in mono 11.5px `#99A2AE` | `12,139 files` / `39.1 GB` |
| Button | `Choose a different folder…` full-width quiet | `Browse Proton Drive…` |
| Helper | `A new empty folder is fine — everything on Proton Drive will be brought down into it.` | `Signed in as you@proton.me · 39.1 GB of 500 GB used` |

**Showing the file count and size for both sides before committing is the point** — it's how
someone notices they picked the wrong folder.

Optional skip-rules prompt `margin-top:34px`: a neutral panel — `⊘` +
`You can tell it to skip things — screenshots, huge exports, scratch folders — now or any time later in Settings.`
+ `Add skip rules`.

Footer: `Nothing is copied or changed until you approve the plan.` + `See what will happen` (primary).

## Step 2 — Nothing gets deleted today

The whole step exists to make one promise, so the promise is the headline.

Centred, `padding:16px 32px 0`. Seam `top:104px; bottom:-56px`:
- `80px` settled hexagon
- `Nothing gets deleted today` 30px/600/`-0.03em`, **masked** (`padding:0 18px`)
- `The first sync only adds. Files you have go up, files on Proton come down, and anything that exists on both sides in different versions is kept as two copies so you can look at them later.`
  14px `#99A2AE` `line-height:1.6` `max-width:600px`, **masked**

Then the two counts, grid `1fr 1fr`, `margin-top:30px`:
- `Going up to Proton` / `128` (38px/600/`-0.035em`) + `files · 1.4 GB` /
  `Files that only exist on this computer.`
- `Coming down to this computer` / `341` + `files · 38.4 GB` /
  **`Needs 38.4 GB free. You have 214 GB.`** — the download side states free space, because that
  is the one thing that can actually fail.

Four fact rows `margin-top:28px`, `padding:11px 2px; border-top:1px solid #16181D; gap:14px` —
`dot · statement (13px) · outcome (12.5px #828B98)`:

| Dot | Statement | Outcome |
| --- | --- | --- |
| `#2E323A` filled | `11,798 files already match on both sides` | `left alone` |
| `2px` ring `#FF6B6B` | `2 files differ on both sides` | `both copies kept — you decide later` `#FF9C9C` |
| `#2E323A` filled | `3 files can't be synced — a socket and two shortcuts` | `skipped` |
| `13px` hexagon outline `#3E454E` `stroke-width:12` | `Nothing will be deleted` | `on either side` |

**The last row is the whole point: zero destructive actions stated as an explicit positive fact,
not a counter reading 0.**

Footer of content: mono 11px `worked out 40 seconds ago · about 25 minutes to finish` +
`See all 471 actions`. Footer bar: `Back` + `Start the first sync` (primary).

## First sync running (600×540)

Seam `top:40px; bottom:110px` with absolute side labels at `top:26px`.
`116px` hexagon, both segments running, numeral `312` (files remaining).
`Bringing everything together` 22px/600 (masked) /
`159 of 471 done · about 17 minutes left` mono 12px (masked).

**Split progress bar** `max-width:400px`: one `3px` track containing two adjacent fills —
warm `12%` then cool `22%` — so the bar shows both directions in one line. Beneath, mono 11px:
`44 sent` (`#FF9F1C`) left, `115 received` (`#22D3EE`) right.

`You can close this window — it keeps going in the background.` 12.5px `#6D7783` centred (masked).
Footer: `nothing deleted · 2 conflicts kept as copies` + `Pause`.

## Consent (600px, after the merge)

`76px` settled hexagon + `Both sides now match` 24px/600 +
`12,480 files, 41.2 GB. Nothing was deleted, and 2 files are waiting for you to pick a version.`

Then the consent panel — `border:1px solid rgba(255,107,107,.3);
background:rgba(255,107,107,.04); border-radius:13px; padding:16px 18px`:
`One thing to agree to before it runs on its own` 14px/600 `#FF9C9C` /
`From now on, deleting a file on either side deletes it on the other. You'll be asked before each one — and you can change that in Settings — but this is how the two folders stay identical.`
12.5px `#C9D0DA` `line-height:1.6`. Then a checkbox row
(`margin-top:14px; padding-top:13px; border-top:1px solid rgba(255,107,107,.16)`): a `17px`
`border-radius:5px` `1.5px #6D7783` box + `I understand deletions travel both ways.` 13px.

Footer: `Syncing stays paused until you agree.` + `Start syncing` (disabled until checked).

**Continuous sync does not begin until this is checked.** That is the trade for having moved the
consent later.

## CLI missing (600px, only on failure)

`34px` warning hexagon + `Proton Drive's command line tool isn't installed` 16px/600 +
`This app drives the official tool rather than talking to Proton directly. Install it once and setup will carry on. Detected Debian — other distributions are in the help.`
+ a command box: mono 11.5px, `$` prompt in `#6D7783`, `sudo apt install proton-drive`,
and a `Copy` button. Then `Check again` (strong) / `Installation help` (quiet).

**The command box is dropped, not made conditional (DEVIATIONS §102, #218).** No distribution
packages `proton-drive`, so the frame above is drawn ground truth kept for the record, not what
ships — `Detected …` stays, but the body it shipped with is the manual path for every distribution
(§102). The instruction below is superseded by that decision and no longer applies:

~~Detect the distribution and show the right command. If detection fails, show the tarball
instructions rather than guessing a package manager.~~
