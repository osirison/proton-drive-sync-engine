# The fidelity harness (F8)

What makes "100% fidelity" checkable rather than a claim. Three gates over the 51 in-scope frames of
`docs/design-v2/Drive Sync.dc.html`.

```
npm run fidelity:extract   # regenerate frames/*.json from the prototype
npm run fidelity           # style + fit gates, then the copy gate
```

## The three gates

| Gate                     | Compares                                                                        | Runs today?                      |
| ------------------------ | ------------------------------------------------------------------------------- | -------------------------------- |
| **style** `assert.mjs`   | every mapped app node's computed styles against the drawn node                  | on whatever carries a `data-fid` |
| **fit** `assert.mjs`     | every full window renders at exactly 1040×764, nothing painting over the footer | yes                              |
| **copy** `copy-gate.mjs` | every fixed string in `ui/copy.js` appears verbatim in the frames               | yes, all 224                     |

## The node key, and why it is a path

`data-fid="<frame-label>:<node-key>"`, where the key is a slash path of tag plus an index among
same-tag siblings — `header/img`, `div[2]/div[0]/span[1]`.

The F8 issue asks that this decision be written down, and offers a hand-maintained mapping file as
the alternative. One argument settles it: **the app's tree is not the prototype's tree and never
will be.** F4 already wraps the app mark in a `<button>` the frames draw bare, and every screen will
diverge further. So a key can only ever be an identifier a human attaches to the app node they judge
to correspond — there is no automatic correspondence to be had. Given that, the identifier should at
least be derivable, diffable and unambiguous, which a list of hand-chosen names is not.

The accepted cost, stated in the issue: a prototype edit that changes structure renames keys, and
the `frames/*.json` diff shows it. That is a review, not a silent break — `assert.mjs` fails loudly
on a `data-fid` naming a key that no longer exists.

The other half of the mapping — _which_ app node is `header/span[2]` — lives in
`src/js/fixtures/frames.js` as each frame's `fids`, and `ui/chrome.js` stamps it.

## Why it renders instead of parsing

The issue says `extract.mjs` "parses" the prototype. It renders it, in the same engine, and that is
the reason the output is usable: `assert.mjs` reads the app off `getComputedStyle`, so the
prototype's side has to come from the same place. A parse gives you `padding:0 20px` where the app
gives you four longhand pixel values, with every cascade, inheritance and default missing.

## Engine

**Chromium, via puppeteer.** The app's real runtime is WebKitGTK and the issue asks for Playwright's
WebKit, which is the right instinct — a gate should run in the engine that ships. It is not usable
here: Playwright's WebKit needs `libicu74` and `libjpeg-turbo8` installed through `sudo`, and a gate
nobody can run locally is a gate nobody develops against. Both scripts take their launcher in one
place; switching is a small change, and CI is the right place to add WebKit alongside Chromium when
the runner allows it.

Every measurement in F1–F4 came from this engine, so the numbers in `DEVIATIONS.md` and the numbers
here are consistent with each other.

## Tolerances

- **Colours** exact. Both sides are already `rgb()`/`rgba()` — no parsing, no rounding.
- **Lengths** ±0.5px.
- **`font-family`** first name only. The fallback stack is a deployment detail.
- **`line-height: normal`** is a wildcard. Resolving it compares font metrics rather than design,
  and F1 established that font metrics move every measurement in this design.
- **Size** is compared as a border box (`getBoundingClientRect`), never as the computed `width`.
  The prototype does not opt into `border-box` and `base.css` opts the app in globally, so the same
  element reports `1000px` in one document and `1040px` in the other while occupying an identical
  1040px on screen. See `BORDER_BOX_INSET`.
- Properties at their CSS initial value are **omitted** from the fixtures and read back as that
  initial (`INITIAL` in `props.mjs`). Lossless, and the difference between a 4.4 MB dump and a diff
  a human will read. Inherited properties are never omitted — "absent" could not be resolved.

## Frame classes

They are not all windows.

| Class          | Count | Asserted                                                                      |
| -------------- | ----- | ----------------------------------------------------------------------------- |
| `window`       | 20    | everything, including the fit gate                                            |
| `dialog`       | 10    | everything at its own size                                                    |
| `compact`      | 11    | everything; note the panel is drawn **362** wide, not 360                     |
| `notification` | 4     | everything except fit — the desktop sizes a banner                            |
| `crop`         | 2     | everything except its own width; drawn at 600 inside the 1040 Settings window |
| `specimen`     | 4     | only the inner artefact; the wallpaper and taskbar are scenery                |

## What this cannot cover, ever

Stated rather than implied, because a gate that seems to cover more than it does is worse than one
that admits its edges:

- **The seven screens with no drawn light frame.** S10 asserts those against `12-light-theme.md`'s
  mapping table, which is prose, not a drawn artefact.
- **Whether an animation looks right.** Only the declaration is comparable — name, duration, delay,
  timing function. A wrong easing that parses is invisible here.
- **Native tray rendering.** Not a webview; it has no DOM. The tray strings are exempted in
  `copy-gate.mjs` with that reason.
- **The desktop's own notification chrome.** Only the banner's content is ours.
- **Motion, focus order and hover states.** The gate reads a static tree.

## State of play

`assert.mjs` reports how many frames carry a `data-fid` and lists the rest every run, so "the gate is
green" can never be confused with "the gate looked at anything". Today the shell's own chrome is
mapped for three frames — 2,109 assertions — and 48 frames are waiting for their screens.

Building the harness before the screens is deliberate: each S-task's definition of done is "my
frames pass", and eleven screens written against no gate at all would be eleven screens to re-check
afterwards. It found six classes of drift in the shell on its first live run.
