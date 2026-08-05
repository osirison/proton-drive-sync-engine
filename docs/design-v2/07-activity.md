# Activity

**Route:** footer nav → *Activity*. **Purpose:** two questions — "did my file make it?" and
"is anything quietly not syncing?"

## What changed and why — two rounds

**Round 1** moved the old ledger here and replaced it with a tide chart (an hour of traffic,
uploads above the centre line, downloads below). It was beautiful when busy and meaningless the
other 95% of the time — a chart of nothing is worse than no chart. That version is kept in the
prototype as `6a` for reference but **is not the spec**.

**Round 2 (this spec)** reframes the screen around what is true on a quiet day:

1. **A file lookup is the first thing on the screen.** "Did my file make it" is the actual daily
   question, it's a search rather than a scroll, and it behaves identically whether zero or four
   hundred files moved.
2. **"n files are never synced" is a permanent section.** Files excluded by a forgotten rule, or
   that the engine can't handle, never announce themselves — today they're a
   `skipped_unsupported` counter. A quiet day is exactly when you'd want to notice them.
3. **The seam took over reassurance:** both sides, counted, agreeing. Meaningful at zero in a way
   a flat line isn't.
4. **History folded in** as a second tab. Two screens of the same log was one too many.

## Layout — the quiet default

```
header 52
title block
search field
seam block: "Both sides agree" + two counts
flex:1: never-synced band + "Last things to move"
footer nav
```

**Title:** `Activity` 26px/600 /
`Nothing has needed to move since 14:32. Both sides matched at the last check, 2 minutes ago.`

**Search field** `margin-top:18px`: `padding:13px 16px; border-radius:11px;
border:1px solid #23262D; background:#0D0E11; gap:11px` — `⌕` 14px `#626B78`,
`Check a file — type any name or path` 14px `#6D7783`, then a `Ctrl F` hint
(mono 11px `#4E5661`, `border:1px solid #23262D; border-radius:5px; padding:2px 6px`).

**Seam block** `margin-top:24px`, seam `top:6px; bottom:0`. Centred and masked:
`52px` settled hexagon (`stroke-width:5.4`) + `Both sides agree` 17px/600.
Then grid `1fr 1fr` `margin-top:18px`:
- `This computer` / `12,480` (30px/600/`-0.03em`) + `files · 41.2 GB` /
  `watched continuously · checked 2m ago` mono 11.5px `#6D7783`
- `Proton Drive` / `12,480` + `files · 41.2 GB` / `next full check in 4m` — right-aligned

**Never-synced band:** `border:1px solid rgba(255,159,28,.28);
background:rgba(255,159,28,.04); border-radius:13px; padding:15px 18px` — `⊘` `#FF9F1C`,
`4 files are never synced` 13.5px/600,
`They sit in your folder but aren't copied anywhere. Two match a rule you wrote; two can't be synced at all.`
12.5px `#99A2AE`, then `Show them` (`#101216`/`#23262D`/`#C9D0DA`).
Warm, not crimson — nothing is at risk, it just isn't backed up.

**Last things to move:** label + right-aligned mono 11px `7 files in the last 3 days`, then three
rows: `glyph · path (mono 12.5px #E8EBF0) · outcome (12.5px #828B98, width:130px) ·
when (mono 11px #6D7783, width:64px, right)`. Footer line:
`Quiet is normal — most days nothing needs to move.` + `All 7 files` + `Sync passes`.

## Looked-up file

Search field switches to active state (`border:1px solid #2E323A`, value mono 14px `#F2F4F7`,
mono 11px `1 match`, and a `22×22` `✕` clear button).

Then, centred on the seam and masked: `52px` hexagon in the matching state, path in mono 15px
`#F2F4F7`, verdict 17px/600 (`Safely on both sides`), and
`Identical here and on Proton Drive since 14:32 today.` 13px `#828B98`.

Two side cards (`border:1px solid #23262D; border-radius:12px; background:#101216;
padding:14px 16px`), the right one right-aligned: size + timestamp in mono 11.5px `#99A2AE`, and
the absolute path in mono 11px `#6D7783` with `word-break:break-all` —
`~/ProtonDrive/docs/spec.md` and `/Drive/RemoteFolder/docs/spec.md`.

**This file's history:** four rows `glyph · sentence · when` —
`Sent to Proton Drive` (today 14:32), `Sent to Proton Drive` (Mon 09:14),
`Both sides had changed — you kept yours` (Fri 16:02, ring dot),
`First brought to this computer` (12 Jul). Then `Open folder`,
`Open on Proton Drive`, and right-aligned mono `linked · id 4c8f…9a21`.

**Pending variant** (600px card): warm-animated hexagon with no numeral,
`On its way to Proton Drive`, `Started 8 seconds ago · 2.8 MB`, a `3px` progress bar, and
`only on this computer so far` in mono 11px.

This needs a per-path history query the daemon may not expose. If it can only answer "current
state", ship the verdict and the two side cards and omit the history block.

## Never-synced detail (600×600 dialog)

`4 files are never synced` 18px/600 /
`They live in your folder but no copy exists on Proton Drive.` Then **grouped by why**:

- `You told it to skip these` (`#FF9F1C`) + `A rule in your settings matches them: *.tmp` →
  rows of `path · size` → `Change this rule` (links to Settings → What to skip)
- `Can't be synced` (`#626B78`) + `Not real files — Proton Drive has nothing to store for them.` →
  `.cache/session.sock` / `a socket`, `projects/current → ~/work/q3` / `a shortcut`

Footer: `Nothing here is at risk — it's just not backed up.` + `Done`.

## Sync passes tab

Tabs are pills: `padding:7px 15px; border-radius:99px` — active
`border:1px solid #2E323A; background:#F2F4F7; color:#0A0B0D; 600`, inactive
`border:1px solid #23262D; background:transparent; color:#99A2AE`.

Sub-line: `18 of the last 20 passes finished cleanly. One failed and retried on its own.`

**Duration chart:** `border:1px solid #1A1D22; border-radius:14px; background:#0D0E11;
padding:16px 20px 14px` — `Last 20 passes` label, then a `56px`-tall
`display:flex; align-items:flex-end; gap:6px` row of `flex:1` bars,
`border-radius:3px`, heights = duration as a percentage. **`#2E323A` for every bar; the one
failure is `#FF6B6B` and the most recent is `#E8EBF0`.** Caption row:
`how long each took · 12:45 onward` / `most recent 14:32`.

**Pass rows:** `padding:12px 2px; gap:14px; border-top:1px solid #16181D` —
`7px dot · outcome (13px) · detail (12.5px #828B98, width:230px) · time (mono 11px, width:44px)`.
`Finished cleanly` with `2 sent, 1 brought here · 1 conflict kept`, `2 sent`,
`4 brought here · 1 move followed`, `nothing to do`.

The failed pass expands in place: tinted `rgba(255,107,107,.05)`, ring dot,
`Couldn't reach Proton Drive` `#FF9C9C`/600, `retried at 14:17 and worked`, and **the exact
daemon string** in a mono 11px `#99A2AE` box (`padding:9px 12px; border-radius:8px;
background:#0A0B0D; border:1px solid #1A1D22`, indented `21px`):
`proton-drive: connection timed out after 60s`. This is the one place the raw string *is* the
useful thing — never paraphrase an error.

Footer: `Only the last 20 passes are kept. Anything older lives in the system log.` +
`Open the system log` (`journalctl --user -u proton-syncd`).

## Details panel (520×460 dialog)

Where the daemon's own words live — the destination of every footer *Details* link. Rows of
`key (mono 11px #6D7783, width:150px) · value (mono 12px #E8EBF0)` separated by
`border-top:1px solid #16181D`: `pending_changes`, `conflicts`, `destructive_actions`,
`skipped_unsupported`, `scan_interval`, `event_stream`, `source` (`status_history`),
`socket`. Footer: `Copy all` + `Open the system log`.

**Every field the old UI printed as a caption under a heading lives here instead.**
