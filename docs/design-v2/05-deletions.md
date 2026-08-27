# Deletions — the approval queue

**Route:** attention band, status chip, or a deletion notification.
**Purpose:** approve or refuse withheld deletions, understanding exactly what is at stake.

## What changed and why

The old screen listed both deletion directions **identically** — same card, same red
`Approve` button, same weight — with a three-sentence paragraph at the top trying to explain
that one direction is permanent and the other isn't, plus `Approve all` / `Deny all` spanning
both. That is the single riskiest thing in the current UI: the two directions are not equally
dangerous.

- Deleting **on Proton** → the file goes to Proton Drive's Trash. Recoverable.
- Deleting **locally** → removed straight from disk, no trash. Gone.

So the seam now sorts by **severity**: permanent on the left, recoverable on the right. Only the
left column carries solid red and a typed confirmation. There is no cross-column "approve all",
because the two columns don't mean the same thing.

## Layout

```
header 52
title block   (26px + 13.5px sub)
flex:1        seam top:0 bottom:14px, grid 1fr 1fr, height:100%
footer "Keep both files" row
footer nav
```

Title: `Two files are waiting to be deleted` /
`They were deleted on one side. Nothing happens to the other side until you say so — syncing carries on around them.`

### Column headers
Left: `8px` filled dot `#FF3B3B` + `Permanent · this computer` (10px/600/`.16em` uppercase,
`#FF9C9C`). Sub: `Removed straight from disk. Not moved to any trash, and not recoverable from Proton.`
12.5px `#828B98`.

Right, mirrored (`justify-content:flex-end`, `text-align:right`): `Recoverable · Proton Drive`
with a `2px` ring dot `#FF6B6B`. Sub: `Moved to Proton Drive's Trash. You can restore it there until the trash is emptied.`

### Item cards
`border-radius:14px; padding:18px 20px; margin-top:16px`.

| | Permanent | Recoverable |
| --- | --- | --- |
| Border | `rgba(255,59,59,.38)` | `rgba(255,107,107,.3)` |
| Background | `rgba(255,59,59,.06)` | `rgba(255,107,107,.04)` |
| Inner divider | `rgba(255,59,59,.2)` | `rgba(255,107,107,.16)` |

Card contents, in this order — **what you'd lose comes first, not the path**:
1. Name 16px/600 + kind in mono 11.5px `#828B98` (`a folder`, `4 KB`)
2. Consequence, 13px `#C9D0DA` `line-height:1.6`, with the loss in `font-weight:600` and the
   destructive tint:
   - `Deleting this removes **1,204 photos, 8.4 GB** from this computer, including everything inside it.` (`#FF9C9C`)
   - `You deleted this on this computer. Deleting it on Proton moves it to **Proton Drive's Trash**, where you can still get it back.`
3. Metadata: `margin-top:14px; padding-top:13px; border-top`, two mono 11px `#828B98` items —
   when it was deleted on the other side, and when you last touched it. **"Last opened Mar 2024" is
   deliberate**: it's the fact that makes the decision easy.
4. Actions.

### Actions — asymmetric on purpose
**Permanent column:** a gate, not a button.
`To delete it, type DELETE below.` (12px `#99A2AE`) then a row: text input
(`flex:1`, `padding:10px 13px; border-radius:9px; border:1px solid rgba(255,59,59,.35);
background:#0A0B0D`, mono 12.5px, `letter-spacing:.1em`, placeholder `DELETE`) and a
`Delete` button that is **disabled until the word matches** (`border:1px solid rgba(255,59,59,.25);
background:rgba(255,59,59,.1); color:#8A5A5A; cursor:default`).

**Recoverable column:** `Move to Proton's Trash`, full-width decision-style button. No gate.

**Both columns**, beneath: `Keep it — put it back on Proton Drive` /
`Keep it — bring it back to this computer` — `border:1px solid #2E323A; background:#101216;
color:#E8EBF0; font-weight:600`, full width, `margin-top:9px`.
**Keep is the visually stronger button in both columns.** It is the reversible one.

### Footer row
`Deletions stay here until you decide. Nothing expires.` 12px `#6D7783`, then `Keep both files`
(`#101216` / `#2E323A` / `#E8EBF0` / 600). The only bulk action, and it is the safe one.

## Two disposal modes

**Everything above describes permanent mode, and permanent mode is no longer the default.** A local
deletion now moves the file to this computer's Trash (`local_delete_mode = "trash"`); permanent
deletion is a setting the user turns back on (08-settings.md, Tab 3). The drawings were made before
that setting existed, so **no frame draws the default** — recorded in DEVIATIONS §100, and the
prose here is the only description of it.

Disposal is a **separate question from gating**. `deletion_policy` decides whether a deletion waits
for approval; `local_delete_mode` decides what happens when it goes ahead. Neither changes the
other's semantics, and an existing config behaves exactly as it did.

What changes on this screen when a local deletion is recoverable:

- The card moves to the **recoverable** column. Severity is `direction === "remote" || disposal ===
  "recoverable"`, so it is derived per card rather than per column.
- The header names what is in the column, not which column it is. A column holding only local
  deletions reads `Recoverable · this computer` /
  `Moved to this computer's Trash. You can restore it from your file manager.`; a column holding
  both reads `Recoverable` / `Each of these can be brought back — from Proton Drive's Trash, or from
  this computer's.` and names no single destination, because its cards have different ones.
- The action is a button, not a gate: `Move to this computer's Trash`. **The typed-`DELETE` gate
  does not arm for a recoverable card** — there is nothing irreversible to confirm.
- The consequence sentence says where the file goes rather than what is lost:
  `You deleted this on Proton Drive. Deleting it here moves it to this computer's Trash, where you
  can still get it back.`
- The permanent column is simply absent when nothing in the queue is permanent — the same rule that
  already hides the recoverable column, not a second one keyed off the config value.

**The severity rule fails closed.** A missing, empty or unrecognised `disposal` — an older daemon,
or a word this build has never heard — reads as `permanent`, so an unknown value keeps the gate
rather than skipping it. The config spelling `"trash"` never crosses the wire and is rejected here
for the same reason: the wire word is `recoverable`.

## Armed confirmation

Full-window takeover. Centred, `padding:0 40px 30px`:
- `104px` hexagon: `fill:rgba(255,59,59,.08); stroke:#FF3B3B; stroke-width:4.6` plus
  `M60 38 L60 66` `stroke-width:6` and `circle cx=60 cy=79 r=3.6`.
- `Delete 1,204 photos from this computer?` — 28px/600/`-0.025em`, `margin-top:26px`.
- Body 14px `#C9D0DA` `line-height:1.65` `max-width:520px` centred:
  `Everything in photos/2019 — 8.4 GB — is removed from disk. It does not go to your trash, and it is already gone from Proton Drive, so there is nothing to restore it from.`
- Confirmation row `margin-top:26px; gap:10px`: a bordered box (`padding:11px 15px;
  border-radius:10px; border:1px solid rgba(255,59,59,.5); background:#0A0B0D`) containing mono
  13px `DELETE` `letter-spacing:.12em` and a `1.5×15px` `#FF6B6B` caret on
  `blip 1.1s`; then `Delete permanently` — `background:#FF3B3B; color:#fff; 600;
  padding:12px 22px; border-radius:10px`. **This is the only place in the app a solid-red fill
  appears.**
- `Keep it — put it back on Proton Drive` beneath, then `Press Esc to cancel.` 12px `#6D7783`.

## Empty state
520×420: `80px` settled hexagon, `Nothing waiting to be deleted` 19px/600,
`When a file disappears from one side, it waits here for you instead of vanishing from the other.`
13px `#828B98` `max-width:320px` centred.

## Compact
Crimson-outline hexagon with the count, `2 files waiting to be deleted`, then one mini-card per
item — severity dot + name + one-line consequence (`1,204 photos gone from this computer, permanently` /
`to Proton's Trash — recoverable`) — then `Review them`. **No approve action in the compact panel.**

## Behaviour
- Deletions never expire and are never auto-applied. Say so.
- Approving removes the row; the column collapses; when both are empty show the empty state.
- The typed word is case-sensitive `DELETE` and clears on blur.
- If the settings deletion policy is *only ask about permanent ones*, the recoverable column is
  absent and its items are applied silently.
