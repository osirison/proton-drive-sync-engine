// Notification fixtures (F9) — the five `11a` frames.
//
// THE COPY DECK DOES NOT CARRY THIS SCREEN, and that is a decision rather than an omission.
// `13-copy-deck.md` §Notifications is one line — "See `11-notifications.md` — all four are quoted
// verbatim there" — so the deck delegates, `ui/copy.js` mirrors the deck and therefore has no
// `NOTIFY` block, and every banner string below is written literally with a `// not in the deck`
// note. The two strings that ARE in the deck are imported, because they are the same string and not
// merely the same words: a banner offering `Open Drive Sync` is offering the tray's row, and letting
// the two drift would be exactly what one copy module exists to prevent.
//
// THE `notification` KEY NAMES THE FRAME CLASS, NOT A BANNER. `tools/fidelity/frame-classes.mjs`
// puts all four non-specimen `11a` frames in one class because the fit gate does not apply to any
// of them — a banner is sized by the desktop's notification chrome, and 520/600px would otherwise
// classify them as dialogs. Two of them are banners (`Grouped`, `Outage`); two are the policy's
// reference sheets (`Rules`, `Settings`) and draw no notification at all. `kind` says which, so S9
// reads the discriminator rather than inferring one from which fields happen to be present.
//
// NOTHING HERE COMES FROM A COMMAND. There is no notification surface in Phase 1 — S9 builds it,
// needing C6 (`notify_policy`, a GUI-LOCAL setting: IMPLEMENTATION-PLAN row 6). `read_config`
// carries no such field and inventing one would pre-empt that design, so `11a Settings` declares
// only which card the frame draws as chosen and leaves the value vocabulary to C6. The three
// triggers behind the two banners are S9's too; a fixture that guessed at their event shape would
// be a second answer nobody had agreed to.

import { MAIN, TRAY, CHROME } from "../ui/copy.js";
import { ago } from "./clock.js";

export const NOTIFICATION_FIXTURES = {
  // ------------------------------------------------------------------------- the two banners ----
  //
  // `at` is an epoch, not the drawn string, because the header's time is a RELATIVE render (`now`,
  // `2m ago`, `14:12` in the three-banner specimen) and rule 3 pins those as an offset. Note what
  // that exposes: `format.js`'s `since(ago(0), "short")` is `0s ago`, and no formatter in the app
  // yields `now` at all. The fixture still pins the epoch — a literal `"now"` would bypass the
  // formatter and hide the gap — and S9 owes the threshold that turns a few seconds into `now`.
  //
  // AND IT IS A GETTER, for the reason plan.js measured on `ui.checkedAt`: a plain `at: ago(0)` is
  // evaluated ONCE, when the module is imported, so the offset ages from there — the harness reaches
  // extraction seconds after load, and a preview tab left open renders minutes later. Invisible at
  // minute resolution and fatal at this one, which is the only register a banner header draws.
  // Reading the property re-runs `ago`, so it is 0 at the moment it renders, whenever that is.

  "11a Grouped": {
    notification: {
      kind: "banner",
      // Several conflicts are ONE banner with the count as the hexagon's numeral, which is the whole
      // point of the frame: `11-notifications.md` §Grouping, coalesced within 30 seconds, never
      // stacked. The mark is the needs-you form in the decision tone — `ui/hexagon.js`'s
      // `needsNumeral`, the same one the `2a Compact needs you` panel draws at 72px.
      icon: { state: "needsNumeral", tone: "decision", numeral: 5 },
      // The notification server's application name, which is the app's short name and not
      // `CHROME.productName` ("Proton Drive Sync"). Not in the deck.
      app: "Drive Sync",
      get at() {
        return ago(0);
      },
      // Both not in the deck. The body counts VERSIONS (ten, from five files changed on both sides),
      // not files — the reassurance is that nothing was overwritten, so the number is deliberately
      // the larger one.
      title: "5 files changed on both sides",
      body: "All ten versions are safe. This usually means another device was offline for a while.",
      actions: [
        // `Go through them` is not in the deck. `Later` is (`MAIN.compact.later`), and it is the
        // same word doing the same job — dismiss without deciding — so it is imported rather than
        // retyped. Neither action is destructive, which is the hard rule this screen exists to
        // state: a banner only ever offers the safe direction.
        //
        // `role`, deliberately not `kind`. A banner's leading action is NOT `controls.js`'s
        // `primary` — that kind is a white fill this surface never uses. Both of these are the
        // notification chrome's own pair (`11-notifications.md` §"Banner spec": a raised
        // `rgba(255,255,255,.06)` on `.14` at 600, then a transparent one on `.1`), which the frame
        // confirms. The fixture says which action leads and leaves the button kind to S9.
        { label: "Go through them", role: "primary" },
        { label: MAIN.compact.later, role: "secondary" },
      ],
    },
  },

  "11a Outage": {
    notification: {
      kind: "banner",
      // Struck through, `#FF3B3B` — the same construction the tray glyph uses for can't-reach, so
      // the banner and the indicator agree at a glance. No numeral: the count is in the body.
      icon: { state: "unreachable" },
      app: "Drive Sync",
      get at() {
        return ago(0);
      },
      // Not in the deck. Voice rule 3 is doing visible work in the body: the sign-in problem is
      // named first and "nothing is lost" closes the sentence, which is the inverse of how an error
      // dialog would write it. `61 changes` is written into the literal rather than pinned as a
      // number, because no command reports it and the string is not the deck's to template.
      title: "Nothing has synced since yesterday",
      body: "Proton Drive is asking you to sign in again. 61 changes are waiting — nothing is lost.",
      actions: [
        // `Sign in` is not in the deck; `Open Drive Sync` is the tray's own row (`TRAY.open`).
        // Both are safe: signing in fixes the cause, opening the app shows the queue, and neither
        // touches a file.
        { label: "Sign in", role: "primary" },
        { label: TRAY.open, role: "secondary" },
      ],
    },
  },

  // ------------------------------------------------------------------- the two policy sheets ----

  /**
   * The reference sheet: four events that interrupt, twelve categories that stay silent, and the one
   * rule about buttons. It draws no banner and has no daemon data behind it — the content IS the
   * policy, so the fixture carries it as the frame draws it.
   *
   * EVERY `why` LINE DIFFERS FROM `11-notifications.md`'s TABLE. The doc writes "The one event where
   * silence can cost files you'd never get back"; the frame draws "The one event where waiting
   * silently could cost you files you'd never get back." All four rows diverge like that, and the
   * hard-rule card is rewritten too. The frame wins (IMPLEMENTATION-PLAN §1.3 rule 2) and these are
   * the frame's words — do not "fix" them back to the prose.
   */
  "11a Rules": {
    notification: {
      kind: "rules",
      // Not in the deck, and the `— 4` is part of the drawn string rather than a count the app
      // formats: writing it as a template would make the fixture compute a displayed string.
      interruptsTitle: "Interrupts you — 4",
      // `dot` is the leading marker's meaning, not a colour. Measured off the frame, and it agrees
      // with the doc's "Icon state" column: filled `#FF3B3B` (irreversible), a 2px `#FF6B6B` ring
      // (a person must decide), filled `#2E323A` (settled — the one good-news trigger), filled
      // `#FF3B3B` again. Which token each becomes is S9's; that the third one is quiet is data.
      interrupts: [
        {
          title: "Something would be deleted permanently",
          why: "The one event where waiting silently could cost you files you'd never get back.",
          dot: "irreversible",
        },
        {
          title: "A file changed on both sides",
          why: "Nothing is lost, but you're now editing two versions of the same thing without knowing it.",
          dot: "decision",
        },
        {
          title: "The first sync finished",
          why: "Once, at the end of a long wait you were told to walk away from.",
          dot: "settled",
        },
        {
          title: "Nothing has synced for a day",
          why: "Not a blip — a real outage, wrong password, or full drive. Silence here is dangerous.",
          dot: "irreversible",
        },
      ],
      silentTitle: "Stays silent — on purpose",
      // Twelve, in the frame's order, which is also the doc's. None are in the deck.
      silent: [
        "every sync pass",
        "every file sent",
        "every file received",
        "folders followed",
        "renames",
        "a single failed pass",
        "retries",
        "scheduled sweeps",
        "skipped files",
        "pause and resume",
        "recoverable deletions",
        "settings saved",
      ],
      // One sentence with a link inside it, so it is three fields rather than one: the frame's own
      // text nodes break exactly here, around an `<a>`. The link's word is the Activity door's name,
      // which the deck does carry (`CHROME.doors.activity`) and which is the door this link opens.
      // The leading space in `before` is drawn — it is what separates "in" from the link.
      activityNote: {
        before: "All of it is in ",
        link: CHROME.doors.activity,
        after:
          ", where you go looking for it. A notification you didn't need is a notification you'll switch off.",
      },
      // The tinted card at the foot. Not in the deck, and the sentence it replaces in the doc reads
      // differently — see the note above.
      hardRule: {
        title: "Never a button in a banner",
        body: "Delete. Discard a version. Approve all. Anything irreversible needs a window where you can see what you're losing — a banner only ever offers the safe direction.",
      },
    },
  },

  /**
   * The `notify_policy` control: three radio cards, the same pattern as Settings' deletions tab.
   *
   * WHICH CARD IS CHOSEN IS THE ONLY STATE HERE, and it is declared as an index rather than as a
   * policy value on purpose. `notify_policy` is GUI-local (IMPLEMENTATION-PLAN row 6 / C6) — no
   * command returns it, `read_config` has no such field, and naming its values here ("needed",
   * "permanent", "never") would hand C6 a vocabulary it never agreed to. The frame tells us one
   * thing: the first card is the chosen one, drawn with a 4px `#F2F4F7` ring on a lifted `#101216`
   * card where the other two sit on `#0D0E11` with a hairline. That fact is `ui.chosen`, per rule 5,
   * and the labels stay in the payload because copy never goes in `ui`.
   */
  "11a Settings": {
    notification: {
      kind: "settings",
      // None of these are in the deck.
      title: "When to interrupt me",
      sub: "Everything else stays in Activity regardless.",
      choices: [
        {
          label: "Only when you need me",
          // `default`, not `recommended` — the deletions tab's badge (`SETTINGS.recommended`) is a
          // different word for a different claim, and this frame draws the other one.
          badge: "default",
          sub: "The four events on the left. Roughly once a week, in a quiet month.",
        },
        {
          label: "Only permanent deletions",
          sub: "The single event that can cost you files. Conflicts wait quietly in the app.",
        },
        {
          label: "Never",
          // The sentence that makes "Never" safe to ship: turning notifications off is not consent,
          // and the deletion queue still holds. `11-notifications.md` is explicit that this choice
          // must not change engine behaviour.
          sub: "The menu bar glyph still changes, and things still wait for you rather than happening on their own.",
        },
      ],
      // Drawn in mono under the cards — the setting's key, shown the way the settings screens show
      // theirs. A config key rather than copy, which is why it is not in the deck.
      settingKey: "notify_policy",
    },
    ui: { chosen: 0 },
  },

  // ------------------------------------------------------------------------------- the mock ----

  // Three banners stacked on a desktop, drawn to show the spacing and the icon states side by side.
  // `frame-classes.mjs` calls it a specimen and asserts through `SPECIMEN_ARTEFACT`, so the frame
  // carries no payload: supplying data for the wallpaper and the top bar would be supplying data for
  // nodes nothing checks, and the three banners it shows are S9's component in three states rather
  // than a fourth thing to describe here.
  "11a In situ": {
    specimen: {
      note: "three banners — permanent deletion, conflict, first sync finished — as S9 draws them, over a desktop mock whose bar, clock and wallpaper are scenery",
    },
  },
};
