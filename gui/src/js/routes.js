// The route table (F4). A route's `kind` decides how it is entered; its `footer` decides whether it
// carries an action bar. The doors themselves are drawn on every screen but the onboarding takeover
// — see FOOTER_ORDER.
//
// Conflicts and Deletions are NOT navigation destinations. They are reached from the attention band,
// the status chip, or a notification.

/**
 * `footer` is the route's ACTION BAR, not its navigation. The frames drew the two as alternatives —
 * 13 frames with doors to 6 with a bar, never both (DEVIATIONS §40) — and a product decision of
 * 2026-08-13 overrides that: the doors are drawn on every screen, under the action bar when there is
 * one. Only the onboarding takeover still has none, because on a fresh machine there is nowhere to
 * navigate to. DEVIATIONS §94.
 */
export const ROUTES = {
  // root — the main screen. Default, and since 2026-08-13 a door of its own: see FOOTER_ORDER.
  main: { kind: "root", footer: "doors", label: "Home" },

  // doors — reachable from the footer nav, in this order, always.
  activity: { kind: "door", label: "Activity", footer: "doors", task: "S5", issue: 184 },
  plan: { kind: "door", label: "Plan a sync", footer: "actionBar", task: "S4", issue: 183 },
  settings: { kind: "door", label: "Settings", footer: "actionBar", task: "S6", issue: 185 },

  // Details is the fourth FOOTER LABEL but the first overlay: 5a/6a draw it as a panel over the
  // screen you were on, not as a destination. Clicking it must not lose your place.
  // `closable` is the ✕, and it is measured per dialog rather than assumed. The two INFORMATIONAL
  // dialogs have one; `saveRefused` does not, because it is asking you to choose between two
  // repairs and a dismiss button in the corner is a third answer the design does not offer. Esc
  // still closes all three — the ✕ is the pointer affordance, not the only way out.
  details: {
    kind: "overlay",
    presentation: "dialog",
    closable: true,
    label: "Details",
    size: [522, 462],
    task: "S5",
    issue: 184,
  },

  // overlays — no door, reached from a band, the chip, a notification or another screen.
  //
  // `presentation` splits them, and it is structural rather than cosmetic — F5 measured that only
  // THREE of the seven are drawn as something floating. A dialog is a standalone surface with no app
  // header and no footer doors, stacked over the screen you were on behind a scrim. The rest are
  // full 1042x766 windows that keep the header and the four doors and replace only the content
  // area, so the body swap F4 already does is exactly right and a scrim would be wrong.
  // DEVIATIONS §57.
  //
  // `size` is the DRAWN box, not the declared one. Four of the ten drawn dialogs opt into
  // `box-sizing:border-box` and six do not, so 520 is drawn 522 while 600 is drawn 600 — there is no
  // offset to apply, only a number to read off the frame. §48a. A null height sizes to content.
  conflicts: { kind: "overlay", presentation: "screen", task: "S2", issue: 181 },
  // THERE IS NO `armed` ROUTE, and F4 provisionally listed one. `4a Armed` is a BODY of the
  // deletions screen — chosen by `screens/deletions.js`'s `bodyOf` from the same queue the cards are
  // drawn from — not a place you can navigate to: it is a confirmation about one specific item, so
  // an id you could reach without naming the item would resolve to a question with no subject.
  // Left in the table it was worse than unused, because `navigate("armed")` would have drawn the
  // "not built yet" placeholder for a screen that IS built. Esc still cancels it; app.js takes the
  // key before `closeOverlay` gets it, which is the one thing being a route would have given free.
  deletions: { kind: "overlay", presentation: "screen", task: "S3", issue: 182 },
  neverSynced: {
    kind: "overlay",
    presentation: "dialog",
    closable: true,
    size: [602, 602],
    task: "S5",
    issue: 184,
  },
  // `7a File pending` — one file, on its way. The odd one out among the dialogs in three ways, all
  // measured: it draws NO title row and no ✕, it carries its padding on the surface itself (which
  // is why it comes out at exactly 600 where `neverSynced` and `details` gain 2px — §48a), and it
  // is CONTENT-SIZED, so `height` is null rather than the 239px the frame happens to be. That
  // number is the sum of what the prototype drew, and this build draws one thing less: the progress
  // bar has no computable value in either direction, so it is omitted and the dialog is shorter.
  //
  // `closable:false` is the ✕, not the exit. Unlike `saveRefused` — the other dialog without one,
  // which withholds it because a dismiss would be a third answer to a two-way choice — this one
  // simply has no corner to put it in. Esc closes it through F4's chain like every other overlay.
  filePending: {
    kind: "overlay",
    presentation: "dialog",
    closable: false,
    size: [600, null],
    padding: "24px 26px 22px",
    task: "S5",
    issue: 184,
  },
  // `8a Save refused` — the daemon would not take the config, and nothing was written. Like
  // `filePending` it draws no title row and carries its padding on the surface itself, which is why
  // it too comes out at exactly 600 where the two dialogs with a head gain 2px (§48a). Its height
  // is content-sized: Phase 1 draws one line of body where the frame draws two (G22 #236).
  //
  // `closable:false` here means what it means on `saveRefused` alone: a ✕ would be a third answer
  // to a two-way choice. Esc still closes it, through F4's chain.
  saveRefused: {
    kind: "overlay",
    presentation: "dialog",
    closable: false,
    tone: "refusal",
    size: [600, null],
    padding: "24px 26px 22px",
    task: "S6",
    issue: 185,
  },

  // The onboarding takeover is an overlay in the routing sense — it covers everything — but it is
  // not opened by the user and cannot be dismissed with Esc. It is entered by the latch below.
  onboarding: { kind: "overlay", takeover: true, footer: "actionBar", task: "S7", issue: 186 },

  // The flow's three dialogs. None is opened by the user and none is closable, so app.js drives all
  // three from its own onboarding state rather than through `openOverlay`/`closeOverlay` — Esc
  // cannot reach them, which is the difference from every other dialog in the table.
  //
  // `9a First sync` is the one 600 that does NOT opt into border-box, so it is drawn 602×542 (§48a).
  // It has a footer of its own, so no padding on the surface. `9a Consent` and `9a CLI missing` both
  // do opt in and both pad the surface, so both are exactly 600 and content-sized.
  firstSync: {
    kind: "overlay",
    presentation: "dialog",
    closable: false,
    tone: "quiet",
    size: [602, 542],
    task: "S7",
    issue: 186,
  },
  consent: {
    kind: "overlay",
    presentation: "dialog",
    closable: false,
    tone: "quiet",
    size: [600, null],
    padding: "30px 30px 24px",
    task: "S7",
    issue: 186,
  },
  // Crimson-edged, like `8a Save refused` — a missing precondition is not a destructive act.
  cliMissing: {
    kind: "overlay",
    presentation: "dialog",
    closable: false,
    tone: "refusal",
    size: [600, null],
    padding: "24px 26px 22px",
    task: "S7",
    issue: 186,
  },
};

/**
 * Footer order. Five labels: the root, three doors and one overlay. **This array is the contract** —
 * the design's checklist has "Footer's doors never move or reorder" as a line item, so nothing may
 * sort, filter or conditionally omit it.
 *
 * `main` leads it by the 2026-08-13 decision: Home is a door of its own, because clicking the lit
 * door no longer returns you here (that toggle was the only way back from Settings and Plan, and it
 * made a tab a switch). The app mark stays a home affordance as well — a second route home is
 * redundant, not wrong.
 */
export const FOOTER_ORDER = ["main", "activity", "plan", "settings", "details"];

/** Overlay routes stack over whatever is underneath; a door replaces it. */
export const isOverlay = (id) => ROUTES[id]?.kind === "overlay";

/**
 * Does this overlay float over the screen, or replace its body?
 *
 * The onboarding takeover answers false deliberately. It covers everything and cannot be dismissed,
 * so a scrim would darken a screen nobody can reach and a focus trap would duplicate the fact that
 * there is nothing else on the window to focus.
 */
export const isDialog = (id) => ROUTES[id]?.presentation === "dialog";

/**
 * THERE ARE NO ROUTE ALIASES ANY MORE, and this note is the record of why there were.
 *
 * `tray.rs` used to emit three ids over `tray-navigate` — `settings`, `conflicts` and `history` —
 * and the third named a screen design-v2 does not have: its two jobs moved into Activity (`6a
 * Activity passes` is the pass history, `7a File lookup` a file's own). Rust does not move when this
 * table does, so `ROUTE_ALIASES` existed to catch that one dead id and keep "View journal" from
 * silently doing nothing.
 *
 * S8 rebuilt the tray, which is the condition this file set for removing it: "only by changing what
 * tray.rs emits". The new tray emits nothing at all — its rows act directly through
 * `commands::tray_action` rather than asking the shell to navigate — so there is no legacy id left
 * to translate, and an alias table with no aliases is a lookup that can only ever be wrong later.
 */

// ------------------------------------------------------------------ onboarding routing (S7) ----

// Carried forward VERBATIM from the deleted state-matrix.js, comment and all, because it is what
// made onboarding reachable on fresh machines (PR #131) and the reasoning is not re-derivable from
// the code. F4 adds the unit test its own comment asked for — see gui/test/onboarding-latch.test.js.
//
// The onboarding takeover is a *latched* decision, not a raw read of the daemon state, because it
// must survive the mid-flow config write: the moment step 2 writes the folder pair, a naive
// `!hasPair` entry condition would flip false and the next 2 s poll would eject the user to the
// unreachable screen. So: once we enter onboarding we STAY until the daemon becomes *reachable* —
// at which point the main screen (with its per-state actions) takes over.
//
// Release on ANY reachable state — `idle`/`running`/`paused` (setup succeeded) AND `authExpired`
// (setup succeeded but Proton sign-in lapsed) AND `failed` (setup succeeded, the first pass did
// not): onboarding can't fix expired auth, and its step-4 status line already promises a hand-off
// once the state moves past `firstRun`, so we must actually hand off to the main screen's
// Re-authenticate action rather than trap the user in the wizard.
//
// `failed` HAS TO BE ON THAT LIST, and it is the half of #246 that is easy to miss. Before that
// state existed a failed first pass derived to `idle`, which released the latch and handed off to a
// main screen saying `Everything is up to date` — the bug. Adding the state without adding it here
// would fix the sentence and break the hand-off instead, trapping someone in a wizard whose two
// steps cannot fix a `proton-drive` binary that is not on the PATH. The main screen carries it, with
// the daemon's own string and a `Try again now`.
//
// Entry has two triggers: (1) `firstRun` — the canonical signal (a reachable daemon that has never
// synced), preserving the original single-hook behaviour; and (2) a genuinely fresh machine — a
// *completed* status poll reports the daemon unreachable AND no folder pair is configured yet.
// `statusPolled` gates this so we never claim "fresh" from the pre-poll default state, and
// `configLoaded` gates it so a daemon configured elsewhere (or a config file not yet read) doesn't
// flash the wizard. Pure and side-effect free so it can be unit-tested.
/**
 * Reachable in any form: the state hands back to the main screen, which surfaces the right
 * per-state action.
 *
 * EXPORTED BECAUSE `app.js` HAD A SECOND COPY OF THIS LIST, and a second copy of a rule is this
 * codebase's most-repeated bug. `render()` gates the sticky `onboardingFailure` on its own
 * `reachable`, and that gate SHORT-CIRCUITS this function — `onboardingFailure ? true : nextOnboardingLatch(…)`
 * — so the two lists disagreeing does not produce a mismatched screen, it makes this one dead code.
 * Which is exactly what happened when `failed` was added here and not there (#246 review): a failed
 * first sync latched the takeover shut, the inverse of the hand-off this arm exists to guarantee.
 * The fix is not to make the copies agree. It is that there is one list.
 *
 * `firstRun` is deliberately absent: the latch treats it as an ENTRY trigger, not a release.
 */
export function releasesOnboarding(daemonState) {
  return (
    daemonState === "idle" ||
    daemonState === "running" ||
    daemonState === "paused" ||
    daemonState === "authExpired" ||
    daemonState === "failed"
  );
}

export function nextOnboardingLatch(prev, daemonState, hasConfigPair, configLoaded, statusPolled) {
  if (releasesOnboarding(daemonState)) return false;
  // Reachable daemon that has never synced: the original firstRun takeover.
  if (daemonState === "firstRun") return true;
  // Fresh machine: a completed poll says the daemon is unreachable AND no folders are chosen.
  if (daemonState === "unreachable" && statusPolled && configLoaded && !hasConfigPair) return true;
  // Anything else (notably `unreachable` right after step 2 wrote the config, or before the first
  // poll): hold whatever we were doing so the flow isn't interrupted.
  return prev;
}
