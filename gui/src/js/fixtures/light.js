// The light theme's datasets (F9) — the eight `12a` frames.
//
// A LIGHT FRAME IS ITS DARK TWIN'S DATASET. Not "similar to" — the same. Seven of the eight are one
// half of a drawn pair, and walking the two node trees in lockstep (the method DEVIATIONS.md
// §"Method" records for the token measurement) gives the same node count, the same tree keys and the
// same text in all seven — measured, not assumed:
//
//   12a Settled light          ← 2a Settled            26 nodes, 12 strings
//   12a Syncing light          ← 2a Syncing            60 nodes, 25 strings
//   12a Conflict light         ← 3a Conflict           75 nodes, 42 strings
//   12a Deletions light        ← 4a Deletions          63 nodes, 33 strings
//   12a Compact settled light  ← 2a Compact settled    12 nodes,  5 strings
//   12a Compact syncing light  ← 2a Compact syncing    36 nodes, 11 strings
//   12a Compact needs light    ← 2a Compact needs you  13 nodes,  6 strings
//
// That is what `12-light-theme.md` means by "Everything else is identical — geometry, type, spacing,
// radii, animation, symbols, copy": light is a token swap, and THERE IS NO SUCH THING AS LIGHT-THEME
// DATA.
//
// SO EACH ENTRY NAMES ITS TWIN RATHER THAN RESTATING IT. `sameAs` is resolved by `frames.js` at
// lookup time, against the registry it has already assembled — so this module imports nothing, names
// twins that live in three other modules, and closes no import edge at all. Restating them would be
// two copies of one dataset that can drift: the day S1 changes what `2a Deletions` withholds, a
// hand-copied `12a Deletions light` keeps the old queue and the light frame quietly stops being the
// frame it was drawn from. There is nothing here for that bug to happen to.
//
// AND SO IS `fids`, SINCE S10 — by the same `sameAs`, for the same reason. It was not: the three
// light COMPACTS were mapped by hand, run against the gate and taken back out, and the four light
// WINDOWS were never mapped at all. DEVIATIONS.md §58b says why, and it was never a fact about the
// mapping: the prototype draws all sixty frames on one dark page, so every `12a` node that sets no
// colour of its own inherits `#F2F4F7`, the extractor recorded that as ground truth, and a
// correctly-light app failed on 142 nodes of it.
//
// `extract.mjs` now records per node which colour properties came from the PAGE rather than from the
// frame, and `assert.mjs` declines to compare those on a `12a` frame and prints how many it declined
// (§91). With the ground truth honest there is nothing left for the asymmetry to protect and one
// thing it costs — a hand-written light table is the second copy of one that already exists, which is
// the shape this build keeps finding bugs in. `check-fixtures.mjs`'s fifth check fails the build if a
// `sameAs` pair's node trees ever stop matching, so the inheritance is checked rather than trusted.
//
// The eighth entry declares its own, because it has no twin to inherit from.
//
// The fixtures also exist so the frames open in the browser preview — `?frame=<label>&theme=light`,
// and the `?frames` index links them that way — which is the half of F9 that is for people.

export const LIGHT_FIXTURES = {
  // ------------------------------------------------------------------------ the four windows ----

  /**
   * Idle, nothing waiting, last pass two minutes ago — the chip reads `idle`, the footer draws the
   * folder pair.
   *
   * The hero's `last synced 2 minutes ago · 12,480 files · 41.2 GB` has no command behind it: the
   * status reply carries the timestamp but neither a file count nor a total size, and S1 owns that
   * line. The twin invents no totals field and neither does this.
   */
  "12a Settled light": { sameAs: "2a Settled" },

  /**
   * Three changes in flight, the pass fourteen seconds old.
   *
   * The footer's `386 MB sent · 1.1 GB received today` is the documented gap G2 (#191) — byte totals
   * per direction, which no Phase-1 command reports. The twin carries none, so neither does this, and
   * the shell draws the folder pair instead.
   */
  "12a Syncing light": { sameAs: "2a Syncing" },

  /**
   * Three unresolved conflicts with the first one open — `3 waiting` in the DECISIONS chip variant,
   * and syncing carrying on around them.
   *
   * The twin's empty `pending_deletions` is load-bearing rather than tidiness, and inheriting it is
   * the point: `chipFor()` tests deletions before conflicts, so one stray entry would turn this
   * frame's chip into the deletions variant while every drawn string still matched. A hand-copied
   * dataset is exactly where that stray entry would appear.
   */
  "12a Conflict light": { sameAs: "3a Conflict" },

  /** Two withheld deletions, one in each direction — `2 waiting` in the deletions variant. */
  "12a Deletions light": { sameAs: "4a Deletions" },

  // --------------------------------------------------------------------- the three compacts ----
  //
  // The panel is its whole frame, so these inherit a `panel` — the arguments `ui/compact.js` takes —
  // rather than a status payload. `12-light-theme.md`'s compact section specifies no copy of its own,
  // which is the same statement as the node walk above: nothing about a panel changes in light except
  // which side of each token ramp it reads.

  "12a Compact settled light": { sameAs: "2a Compact settled" },
  "12a Compact syncing light": { sameAs: "2a Compact syncing" },
  "12a Compact needs light": { sameAs: "2a Compact needs you" },

  // ------------------------------------------------------------------------------ the sheet ----

  /**
   * The eighth frame is the one with no twin, and the only `12a` that is not a product surface:
   * `frame-classes.mjs` calls it a specimen and `SPECIMEN_ARTEFACT` narrows it to the glyph alone,
   * because the card the glyph sits on is drawn DARK even in this frame. It says in one sentence what
   * the whole screen doc says at length — state is carried by fill, so the inverted glyph needs
   * nothing re-specified — and there is nothing to feed it.
   */
  "12a Tray light": {
    specimen: {
      note: "the 14px tray glyph inverted to near-black on a light panel (ui/hexagon.js, placed by S8); its surrounding card is drawn dark and is not product",
    },
    /**
     * The two facts the frame pins, and they are the only two it has. `renderTrayLight` in
     * `preview.js` builds the strip around them.
     *
     * 14, not the sheet's 20: `10-tray.md` gives a range and every mark on `10a Glyph states` is
     * drawn at 20, but this frame draws its one glyph at 14 — the smallest size in `STROKE`, and the
     * point of the frame is that the form survives being both inverted AND that small.
     *
     * `needsYou` because it is the form with the most to lose from inversion: the other four are
     * outlines, and this one carries a FILLED centre, which is the property `10-tray.md` calls
     * load-bearing ("state is carried by fill rather than hue"). A settled outline would prove
     * nothing about a light panel.
     */
    trayStrip: { state: "needsYou", size: 14 },
    /**
     * THREE SLOTS, AND THE ISSUE ASKS FOR EXACTLY THREE. `frame-classes.mjs` calls this a specimen
     * and `SPECIMEN_ARTEFACT` narrows it to "only the 14px glyph is product — its own card is drawn
     * dark". So the card, the light strip, `Activities` and the three status marks beside ours carry
     * no slot: they are scenery, and mapping them would assert a mock.
     *
     * The names are `glyphFids`' — the same three nodes mean the same three things — but as strings
     * rather than factories, because this frame draws one mark where the sheet draws ten. `fid()`
     * passes its extra arguments only to a factory, so `stampGlyph` drives both.
     */
    fids: {
      glyph: "div[0]/span[2]/svg",
      glyphPath: "div[0]/span[2]/svg/path",
      glyphCircle: "div[0]/span[2]/svg/circle",
    },
  },
};
