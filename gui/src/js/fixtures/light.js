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
// WHAT IS DELIBERATELY ABSENT: `fids`. The three light COMPACTS were mapped, run against the gate and
// taken back out — DEVIATIONS.md §58b: the prototype draws all sixty frames on one dark page, so
// every `12a` node that sets no colour of its own inherits `#F2F4F7` and the extractor records that
// as ground truth, against which a correctly-light app fails on 142 nodes. The four light WINDOWS
// were never mapped at all (`SHELL_FIDS` carries only the three `2a` labels). Both facts belong to
// S10, which owns light and needs the answer for the seven screens with no drawn light frame; this
// module does not re-litigate either.
//
// That absence is enforced rather than trusted: `resolveFixture` inherits a twin's DATA and never its
// `fids`, so mapping `2a Settled` in S1 cannot silently map `12a Settled light` too. It is the one
// asymmetry in `sameAs`, and it is there for this paragraph.
//
// The fixtures exist so the frames open in the browser preview — `?frame=<label>&theme=light`, and
// the `?frames` index links them that way — which is the half of F9 that is for people. What they
// feed today is the shell F4 built: header chip, footer, doors. The heroes and bodies below are not
// missing data, they are unbuilt screens, exactly as in the dark twins.

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
  },
};
