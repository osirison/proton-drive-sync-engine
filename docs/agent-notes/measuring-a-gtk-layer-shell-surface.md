---
trigger: gtk-layer-shell, layer surface, zwlr_layer_shell_v1, is_supported, init_layer_shell, set_layer_shell_margin, set_size_request, panel invisible, spectacle, skip_taskbar wayland, frameGeometry, empty caption, resourceClass, focus-in-event, focus-out-event, WindowEvent::Focused, panel does not hide on blur, queue_resize, outer_size
depends_on: gui/src-tauri/src/panel.rs, gui/src-tauri/src/lib.rs, packaging/, setup.sh, .github/workflows/ci.yml
recorded: 2026-08-30 (corrected the same day, after an adversarial re-measurement of cfde6d0;
  corrected again 2026-08-31 — citations de-numbered, two inferences downgraded, section 7's
  own account of where each finding landed fixed; corrected again 2026-09-16 — section 3 reversed
  (a mapped layer surface DOES resize, #385/#401), a read-back trap added to section 1, and a new
  section 8 recording that `hide()` unmaps the layer surface)
---

# Measuring a layer surface on Plasma 6: KWin lists it, `spectacle` cannot capture it

Preconditions for measuring the tray panel's layer surface (`panel.rs::promote_to_layer_surface`),
all on Plasma 6 / Wayland. **The first version of this note had two of them backwards**, and the
corrections live here rather than in a second file: a note that sends the next session to a screen
capture for a number KWin answers directly costs that session an afternoon.

## 1. KWin DOES list a layer surface — the earlier "it is absent" reading was a broken filter

An unfiltered `workspace.windowList()` dump contains the layer surface. Exactly:

```
class=layerprobe cap='' geom=400,300 362x442 skipTaskbar=true skipPager=true
```

It is managed and it has geometry. **The caption is empty**, because wlr-layer-shell has no title
request — and that is what the earlier reading tripped over. The dump in
`kwin-read-a-window-app-id.md`, under its heading *"The same reading proves a toolkit call inert, if
you run a control"*, filters on the caption — the line
`if (String(w.caption).indexOf("SKIPPROBE") === -1) continue;` — so the surface never matched and
was recorded as not there at all. Dump unfiltered, or filter on `resourceClass`, which reads
`layerprobe` above.

**The panel's `resourceClass` is not the app's `app_id`, and filtering on the wrong one reads as
"the panel is not there" — a second trap of the same shape as the caption one, and it cost 62 blind
samples before the filter was the thing under suspicion rather than the panel.** The panel's layer
surface reports `resourceClass` `proton-sync-gui`; the main window carries the app_id this app
adopts, `app.protondrivesync.engine`. A KWin dump filtered on the app_id shows the main window only.
That the two classes differ is MEASURED; that the string is `argv[0]` is INFERRED.

**Cite that file by quoted text, by heading, or by the identifier being pointed at — never by line
number.** These three references used to carry line numbers, and the change that inserted the
caption-filter warning into that file moved every one of them by eight lines, so all three landed on
unrelated text. A line number is the one form of citation a change to the cited file invalidates
silently, and these two notes are read together precisely when someone is editing both.

Two things follow, and only the second of them is a finished answer.

**The taskbar fix reads as `skipTaskbar=true` on a MANAGED window** — not as "there is no window to
carry it", which is what this note used to say and which inverts the reading that actually works.
The fix for #351 is real, and what was observed is the property being true on a window KWin manages.
**What sets it was not verified, and no mechanism should be written here until it is.** The control
that would settle it is the same layer-surface probe issuing no `skip_taskbar` call at all: still
`true` means something other than that hint sets it, `false` means the hint reaches the surface by
some path. That probe was not run.

**Geometry comes from KWin, not from pixels.** `geom=400,300` is the requested logical position,
read straight back from `frameGeometry` — so the #370 position fix is confirmed by one scripting
call and **a screen capture was never needed**. The capture-and-scan-for-a-colour recipe this note
used to prescribe was adopted only because of the filter bug above. The working method is already
written down in the sibling note, in the same section as the filter above: its dump reads
`var g = w.frameGeometry` — the line commented `// for a positioning probe` — and the paragraph
beginning *"Positioning is the same probe with `w.move(x, y)`"* is that reading applied to a
toplevel. Use it, rather than the pixels this file used to send you to.

### Units on the read-back

The margin was requested as `400,300` **logical** and `frameGeometry` answered `400,300` on a
scale-2 display — no scale conversion on the way out of KWin. That is the observation on this
machine, not a general rule about the scripting API; a second scale factor would be needed to make
it one. Which space a number is in still matters, because `place()` works in **physical** (the click
arrives physical) while `set_layer_shell_margin` takes **logical**: at scale 2 the same surface sits
at physical `800,600`, which is what a pixel measurement would report if one could be obtained —
see section 2 for why one cannot. That conversion is the first thing to check when the arithmetic
looks right and the window is in the wrong place.

## 2. `spectacle` does NOT capture a layer surface

With KWin listing the surface and reporting its geometry at that same moment (section 1), a
full-screen `spectacle -b -n -f -o /tmp/s.png` showed nothing of it.

**Read that premise for exactly what it is.** This section used to open "with the surface provably
rendered", which treats a window-list entry carrying geometry as an observation that pixels reached
the screen. It is not one: it is the compositor's own bookkeeping about a surface it manages. The
paragraph below is precisely the finding that the two cannot be equated here, so opening with them
equated made an inference into a premise.

This note previously claimed the opposite, and explicitly ruled out the capture-limitation
explanation that the re-measurement restores. That mistake rested on the filter bug: with the window
list wrongly reading empty, "no window" and "no pixels" looked like one fact about visibility rather
than two separate readings.

So a colour scan over a capture cannot answer "is the layer surface on screen". A negative proves
nothing, and there is no positive to be had.

**And nothing here puts anything in its place.** This paragraph used to end "read KWin instead",
which nominates a method nothing tested for that question. The undrawn state never reproduced
(section 5), so no dump was ever taken against a surface known not to be drawn, and what KWin would
report for one is unknown. Section 1's entry evidences that the surface is MANAGED, which is what it
measured; whether such an entry differs for an undrawn surface is exactly the untested part, and
nothing here says KWin cannot answer it either. Disqualifying the capture is the finished
part; **this change leaves no validated method for the visibility question at all**, and a paragraph
that swapped one method for another would have hidden that. Geometry is a different question, and
KWin does answer that one (#370, section 1).

## 3. A MAPPED layer surface DOES resize — the earlier reading here was wrong

**Refuted against the shipped binary** (#385, refutation measured 2026-09-16). Plasma 6 / KWin 6.7.5 Wayland,
`main` @ `04d615f`, `target/debug/proton-sync-gui`, run against an isolated `XDG_RUNTIME_DIR` with
no daemon socket, panel opened only by `org.kde.StatusNotifierItem.Activate`, read back through
KWin's scripting API:

```
t=...679478  1415,606 362x442   <- opens at HEIGHT
t=...679763  1415,746 362x302   <- corrected, ~365 ms after Activate
... 81 further samples, all 362x302
```

```
[19:10:51.681829]  -> zwlr_layer_surface_v1#53.set_size(362, 442)
[19:10:51.682045]     zwlr_layer_surface_v1#53.configure(12756, 362, 442)   <- MAP BOUNDARY
[19:10:51.923291]  -> zwlr_layer_surface_v1#53.set_size(362, 302)    +241 ms, AFTER the map
[19:10:51.924571]     zwlr_layer_surface_v1#53.configure(12760, 362, 302)
```

So the PROTOCOL's `zwlr_layer_surface_v1.set_size` does reach the wire after the map and KWin does
honour it. **Read which `set_size` that is.** It is the request gtk-layer-shell derives from the GTK
size REQUEST, not tao's `set_size`/`gtk_window_resize` — those are toplevel calls and section 5's
claim that they never reach a layer surface is UNAFFECTED, re-measured here: `gtk_window_resize` on
its own still puts nothing on the wire and moves nothing. The two share a name and nothing else, and
running them together is how a comment in `panel.rs::resize` was written wrong and caught in review.
The correction lands
250-500ms after the panel appears, on the FIRST open — no reopen needed — and again on every later
state change: the same run's two mid-life changes measured landing correctly on the wire but showing
up 63px, then 48px, out of position — a *different* fault (#401: the re-place read the panel's
height back from `outer_size()`, a cache the compositor had not caught up on), now fixed and pinned
by `panel.rs`'s `regression_401_panel_origin_derives_its_size_from_the_height_it_is_given` test,
which carries both numbers.

**What reproduced the old reading, and what it does and does not establish.** A scratch GTK3 probe
replicating `promote_to_layer_surface` — tao's exact sizing calls, tao's default `GtkBox`, a real
`WebKitWebView` packed with `pack_start` — resized correctly in five variants. The only construction
that reproduced the two lines below exactly (GTK allocation unchanged, KWin geometry unchanged,
`queue_resize()` no help, nothing on the wire) is one where a widget UNDER the window carries its
own size request greater than or equal to the old height:

```
RZ2 issued resize -> 362x250
RZ2 after alloc=362x442        <- GTK unchanged
KWin  geom=400,300 362x442     <- compositor unchanged
```

The shipped stack does not have such a widget. `wry` 0.55.1 calls `set_size_request` on the webview
only when the container is a `GtkFixed` (`src/webkitgtk/mod.rs:615`), and `tauri-runtime-wry` 2.11.4
builds both webview kinds into `window.default_vbox()` — a `GtkBox` — on Linux. `panel.rs`'s own
call is the only `set_size_request` in `gui/src-tauri` or `gui/gui-core`, and it is on the
`GtkWindow`, not under it.

Three readings of that RZ2 measurement, kept apart because nothing here separates them:

  * **MEASURED** — a pinned child widget reproduces the two lines above exactly. That this was
    RZ2's actual cause is **UNDETERMINED**: that probe's own source no longer exists to check.
  * **INFERRED** — RZ2 may instead have blocked the GTK main loop after its two calls, so GTK never
    processed the resize and nothing reached the wire. Not measured either way.
  * **Reading `outer_size()`/GTK's allocation too early is not sufficient on its own.** It explains
    RZ2's GTK line — every construction here reproduces that synchronously, GTK re-allocates a few
    ms later regardless — but not its KWin line, since a KWin dump costs about 0.6s and every dump
    taken since has read the new size.

**This measurement cannot distinguish "this section was wrong when it was written" from "the stack
changed since 2026-08-30."** Nothing here settles that, and this rewrite does not adjudicate it
either — it corrects what the note claims, not why the old claim was made. Versions, so a future
divergence can be dated: `main` @ `04d615f`, Plasma 6 / KWin 6.7.5, tao 0.35.3, gtk-layer-shell
0.8.2, wry 0.55.1, tauri-runtime-wry 2.11.4.

The consequence for the shipped panel is therefore the opposite of what this section used to say:
the first-paint height correction lands while the panel is visible, on the first open, and the panel
keeps correcting itself on every later state change too.

## 4. A layer surface delivers NO focus events to GTK

The probe logged **no `focus-in-event` and no `focus-out-event`** — the two GTK signals tao converts
into `WindowEvent::Focused`. Neither fired, **not even at map**.

So on this path `panel::mark_focused` never runs, and the blur-to-hide arm in
`gui/src-tauri/src/lib.rs` (the `Focused` arm keyed on `panel::LABEL`) is dead in both directions:
nothing sets the flag, and nothing would deliver the blur that reads it. **The panel does not
dismiss by looking away.** Recorded, not fixed.

Anything else built on `WindowEvent::Focused` for this window is dead for the same reason. That is
the general form of the precondition: on a layer surface, focus-derived state has no writer.

## 5. What this measurement did NOT settle

Two claims are open. Neither is refuted; neither is re-established. They are written out at length
because the version of this note that stated them as facts is exactly what the re-measurement had to
undo.

**"The panel is invisible without `set_size_request`" did not reproduce.** Against tao's exact
construction path the surface appeared without it. The original reading came from a different
construction and the difference between the two was never isolated, so the call is neither
vindicated nor convicted.

**What would settle it is one experiment in two halves, and the second is worthless without the
first.** (1) Run the two constructions side by side — the one the original reading came from, and
tao's exact path, where the invisibility did not reproduce — and isolate what differs. (2) In
whichever construction reproduces the invisibility, build the window that way (same widget
hierarchy, same realize order) and toggle only this call. Both halves need a way to tell a drawn
surface from an undrawn one and this change validated none: `spectacle` captures no layer surface at
all (section 2), and nothing shows a KWin read discriminates the two, because the undrawn state
never reproduced for anything to be read against. **Finding that read-out method is part of the
experiment, not a step that precedes it.** The same plan, in the same words, is at the
`set_size_request` call in `promote_to_layer_surface`.

**The call stays** in `promote_to_layer_surface`: it is harmless, and removing a line on the
strength of one non-reproduction is how an invisible-panel afternoon happens twice. Two of the
surrounding facts were not challenged and still stand: a layer surface anchored to two
OPPOSITE edges is sized by the compositor (a probe anchored to all four was configured `1920x1034`,
the screen minus the panel's exclusive zone), and gtk-layer-shell turns the GTK size REQUEST into
`zwlr_layer_surface_v1.set_size` while the toplevel calls — `gtk_window_set_default_size`, Tauri's
`inner_size`, `WebviewWindow::set_size` — do not reach it. **The rest of the original explanation is
not re-established**: that anchored to one CORNER nothing derives a size, so the client must send
one. That half predicts precisely the blank screen which did not reproduce, and how the reviewer's
probe got a size without the explicit request was not determined — so do not repeat it as a fact
either. This paragraph used to end *"What the size request does not buy in any case is a resize while
mapped: section 3"* — **refuted along with section 3** (#385, measured 2026-09-16): the size request
is exactly what buys a resize while mapped. The half that stands is the one above it, and it is
narrower than it looks: the TOPLEVEL calls still reach nothing.

**Whether a CLICK grants keyboard focus under `KeyboardMode::OnDemand` — and so whether Esc still
dismisses the panel — is not established by this measurement.** `panel.rs` sets `OnDemand`, whose
stated purpose is that the surface takes keyboard focus when it is clicked. Section 4's probe
observed no focus events *including at map*, which is the expected result for `OnDemand` with no
click: the measurement never issued one, so it cannot speak to the clicked case. This note therefore
says neither that Esc works nor that it is broken. What would settle it is a probe that clicks the
surface and then reports whether a key press is delivered to it.

## 6. A nested compositor is not available here

`kwin_wayland --width ... --socket=...` does not start in this environment (no socket, no log), so
a probe cannot be isolated from the real desktop. That turns out to cost less than it looked:
every geometry reading above came from KWin's scripting API against the live session, which reads
no pixels, so no capture of the user's screen was needed — and per section 2 there is no capture
fallback to fall back to in any case.

## 7. `panel.rs` was corrected in the same change — and the rule if the two ever diverge

What stood here was that the correction "touched no other file", followed by four strings in
`panel.rs` to distrust. Both halves were wrong. The change that corrected this note corrected
`panel.rs` in the same pass, across the packaging and doc-site dependency lists as well; and the
four strings had already been rewritten out of `panel.rs` by that same change, so the list sent the
next reader hunting for text that no longer existed. A list of quotations is the wrong form for
this in any case: it decays the moment either side is edited, and its decay is silent.

The two agree today, and the agreement is not a block of this text copied into one place there.
**Each measurement is written beside the code it constrains** — wherever a reader would otherwise
draw the wrong conclusion — which is why no single comment in `panel.rs` carries all of it. This
paragraph used to say sections 1-4 were "written into `panel.rs`'s module doc and into
`promote_to_layer_surface`", and section 3 is in NEITHER, which is the worst thing for the one
section whose job is telling a future reader which side to trust. Where each landed today, by
identifier rather than by line:

  * **1** (KWin lists the surface; `skipTaskbar=true` as an outcome with no established cause;
    geometry off `frameGeometry`) — the module doc, and `promote_to_layer_surface`'s doc comment.
  * **2** (`spectacle` captures nothing of a layer surface, and nothing replaces it) — inside
    `promote_to_layer_surface`'s BODY, in the paragraph at the `set_size_request` call. The module
    doc points at it rather than repeating it.
  * **3** (a MAPPED layer surface DOES resize — this note's own earlier claim was refuted by #385)
    — `resize_layer_surface`'s doc comment, the `HEIGHT` constant, the `ANCHOR` static, and
    `resize`'s body. Same four places as before the correction, not moved: each is a place a reader
    could otherwise still conclude, from the wording this note used to carry, that the correction
    lands only after a close and a reopen. **AND FOUR THIS LIST DID NOT NAME**, which is how they
    kept the refuted claim after the rest was corrected and why they are named now:
    `commands::resize_tray_panel`'s doc, `app.js`'s `reportTrayHeight` doc, and — found only by a
    reviewer, not by the sweep — `gui/tools/fidelity/README.md`'s squeeze-gate section and the same
    comment above `SQUEEZE_VIEWPORT` in `assert.mjs`. The sweep missed those two because it grepped
    the phrasings the note itself used, and they had worded the same claim differently ("a mapped
    one ignores the resize, so the height lands at the next open"). Sweep by the CLAIM, not by this
    note's words for it.
  * **4** (no focus events at all) — `promote_to_layer_surface`'s body at
    `KeyboardMode::OnDemand`, in full, which every other mention in that file points at; plus the
    module doc's dismissal bullets and `mark_focused`.
  * **5**'s two open questions — open on both sides, which is the part most worth preserving,
    because agreement there is agreement that something is UNKNOWN, not a second vote for a fact.
    The invisibility one is at the `set_size_request` call and the Esc one at
    `KeyboardMode::OnDemand`, each stating the same next step this note states.
  * **8** (`hide()` unmaps the layer surface; every `show` rebuilds it) — NOTE-ONLY, deliberately:
    `hide()`'s own doc in `panel.rs` is a one-liner ("Hide it, from anywhere.") that claims nothing
    about the layer surface, so there is no wrong belief for a code reader to form there without
    this. Recorded here because it bears on #384's untested `hide()` caveat, not because `panel.rs`
    needed correcting.

If that list is what decays next, prefer the rule to the list: grep `panel.rs` for the finding, not
for the place this paragraph last said it was.

**If they ever diverge again, one of them was edited alone, and which is later is not settled by
which file it lives in.** Settle it on the evidence a claim cites: a side that names a probe and
what that probe read beats a side that names none, and a claim neither side can trace to a
measurement is one to re-run rather than to arbitrate between. This note is where a measurement is
recorded first, so a reading here that `panel.rs` lacks is usually the newer one — but a `panel.rs`
comment citing a probe this file does not mention is evidence this file is behind, and the fix is
to record it here, not to delete it there.

Unchanged and not in question: `skip_taskbar` and `set_position` reach GTK as
`gtk_window_set_skip_taskbar_hint` and `gtk_window_move`, both X11-only and both silently discarded
on Wayland; Mutter does not implement `zwlr_layer_shell_v1`, so on GNOME `is_supported()` is false
and the panel stays an ordinary toplevel with both original bugs; and on X11 `is_supported()` is
false, the two hints work, and that path is untouched by any of this.

## 8. `hide()` unmaps the layer surface; every `show` rebuilds it

MEASURED, same session as section 3's refutation. `hide()` puts `zwlr_layer_surface_v1.destroy()`
and `wl_surface.destroy()` on the wire, and KWin drops the panel from `workspace.windowList()`
entirely — not hidden, gone. GTK reports `visible=false mapped=false`.

Every `show` after that creates a NEW `zwlr_layer_surface_v1`: the GTK window is promoted to a layer
surface once (`promote_to_layer_surface`, which cannot run a second time — it requires an unrealized
window), but the Wayland object it wraps is destroyed and rebuilt on every hide/show cycle, carrying
the last reported height and margin in its pre-map requests.

This bears on issue #384, which names the untested `hide()` path as a caveat.

## Scratch probes

`examples/` is shipped packaging (`cargo-run-example-scratch.md`). Use `cargo new` in the session
scratchpad with `gtk = "0.18"` and `gtk-layer-shell = { version = "0.8.2", features = ["v0_6"] }`.
