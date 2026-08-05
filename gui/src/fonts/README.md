# Bundled typefaces

Design-v2 is **Instrument Sans** for anything a person reads and **IBM Plex Mono** for anything the
machine owns — paths, globs, config keys, remote ids, byte counts, timestamps, daemon error strings.
The mono is the signal that you are looking at ground truth (`docs/design-v2/01-foundations.md` §2).

Both are committed here as Latin `.woff2` and `@font-face`'d in `../styles/base.css`. They are not
resolved from `node_modules` at runtime: Tauri serves `gui/src` **raw** (`"frontendDist": "../src"`,
no bundler, no build step) and `node_modules` is not shipped, so a font that is not committed in
this directory does not exist in the app. The Tauri CSP (`font-src 'self' data:`) forbids external
font hosts, and the app must render offline, so there is no CDN fallback either.

Both are **SIL Open Font License 1.1** — `OFL-instrument-sans.txt`, `OFL-ibm-plex-mono.txt`, copied
from the upstream packages. The three packaging surfaces declare them (`packaging/debian/copyright`,
`packaging/rpm/*.spec`, `packaging/arch/PKGBUILD`), because Tauri compiles these files into the
`proton-sync-gui` binary.

## Which weights, and why these

| Family          | Bundled            | Measured use in the prototype                        |
| --------------- | ------------------ | ---------------------------------------------------- |
| Instrument Sans | 400, 500, 600, 700 | 400 × 504 nodes · 600 × 244 · 500 × 13 · **700 × 0** |
| IBM Plex Mono   | 400, 600           | 400 × 352 · 600 × 19 · **500 × 0**                   |

The counts are measured node-by-node over `docs/design-v2/Drive Sync.dc.html`, not taken from prose.
`01-foundations.md` §2 says "weights in use: 500, 600, 700 (sans); 400, 600 (mono)" and adds
"nothing is 400-weight sans except long body paragraphs" — **the frames disagree**: sans 400 is the
single most common weight in the design. Bundling the prose list would have left the majority of the
UI rendering in `system-ui`, with every measurement the F8 fidelity gate later asserts taken against
the wrong typeface. 700 is bundled anyway because §2 names it: a real face costs ~17 KB and its
absence would be invisible — `font-weight` computes to 700 whether or not a 700 face exists, so the
style gate would pass on a synthetic bold. Mono 500 is not bundled; nothing asks for it. Recorded in
`docs/design-v2/DEVIATIONS.md`.

Italics are not bundled. The prototype's 30 `<em>` elements all carry `font-style:normal`.

## Subsets

`latin` **and** `latin-ext`. Every frame is ASCII, but this app renders arbitrary **filenames**, and
`unicode-range` means the ext face costs nothing until a path actually contains a character outside
the latin subset — at which point the alternative is a system-font glyph in the middle of a
filename.

Neither subset covers the Unicode symbols in `01-foundations.md` §8 (`→` `←` `⇄` `↷` `✕` `⊘` `⌕`
`⋯` `▲`): `latin` includes U+2191/U+2193 but not the horizontal arrows, so those glyphs come from a
system fallback today. §9.4 of the implementation plan already proposes replacing the whole Unicode
set with a line icon set as its own issue; that is the fix, not a wider subset.

## Updating

```
npm --prefix gui install                 # after a @fontsource bump
npm --prefix gui run fonts:sync          # re-vendor
npm --prefix gui run fonts:sync -- --check   # verify committed == node_modules
```

`fonts:sync` is deliberately **not** part of `npm run check`. Upgrading a typeface is not a
dependency bump — new metrics move every measurement in `docs/design-v2` and re-baseline the
fidelity frames. A `@fontsource` version bump that nobody syncs leaves the app on the font the
frames were measured against, which is the safe outcome. Sync when you mean to move, and expect to
regenerate the F8 fixtures in the same PR.
