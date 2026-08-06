// The route table (F4). Three kinds of route, and the difference is structural rather than
// cosmetic — it decides what the window's footer is, which is the single biggest shape change from
// the v1 sidebar build.
//
// Conflicts and Deletions are NOT navigation destinations any more. They are reached from the
// attention band, the status chip, or a notification. That is why the footer has four labels and
// only three of them are doors.

/**
 * MEASURED, not read: every in-scope 1040 frame carries EITHER the four doors OR a footer action
 * bar — 13 frames to 6, never both, never neither. `02-shell.md` says the doors "never move and
 * never change order, on any screen, in any state", which reads as *present everywhere*; the frames
 * say the action bar replaces them on the screens that commit something. See DEVIATIONS.md §40.
 *
 * The consequence is a real product constraint, not a layout detail: on Settings, Plan and
 * onboarding there is no navigation at all, so the action bar's own secondary button and the app
 * mark are the only ways out. That settles half of IMPLEMENTATION-PLAN.md §3.3's open question by
 * elimination — the app mark as a home affordance is not optional.
 */
export const ROUTES = {
  // root — the main screen. Default, and no door of its own.
  main: { kind: "root", footer: "doors" },

  // doors — reachable from the footer nav, in this order, always.
  activity: { kind: "door", label: "Activity", footer: "doors", task: "S5", issue: 184 },
  plan: { kind: "door", label: "Plan a sync", footer: "actionBar", task: "S4", issue: 183 },
  settings: { kind: "door", label: "Settings", footer: "actionBar", task: "S6", issue: 185 },

  // Details is the fourth FOOTER LABEL but the first overlay: 5a/6a draw it as a panel over the
  // screen you were on, not as a destination. Clicking it must not lose your place.
  details: {
    kind: "overlay",
    presentation: "dialog",
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
  deletions: { kind: "overlay", presentation: "screen", task: "S3", issue: 182 },
  armed: { kind: "overlay", presentation: "screen", task: "S3", issue: 182 },
  neverSynced: {
    kind: "overlay",
    presentation: "dialog",
    size: [602, 602],
    task: "S5",
    issue: 184,
  },
  saveRefused: {
    kind: "overlay",
    presentation: "dialog",
    tone: "refusal",
    size: [600, null],
    task: "S6",
    issue: 185,
  },

  // The onboarding takeover is an overlay in the routing sense — it covers everything — but it is
  // not opened by the user and cannot be dismissed with Esc. It is entered by the latch below.
  onboarding: { kind: "overlay", takeover: true, footer: "actionBar", task: "S7", issue: 186 },
};

/**
 * Footer order. Four labels, three doors and one overlay. **This array is the contract**: the
 * design's testing checklist has "Footer's four doors never move or reorder" as a line item, so
 * nothing may sort, filter or conditionally omit it.
 */
export const FOOTER_ORDER = ["activity", "plan", "settings", "details"];

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
 * v1 route ids that outlived the screens they named, and where they land now.
 *
 * `tray.rs` emits three ids over the `tray-navigate` event, and it is Rust — it does not move when
 * the frontend's route table does. `settings` and `conflicts` still resolve; `history` does not,
 * because design-v2 has no History screen. Its two jobs both moved into Activity: `6a Activity
 * passes` is the pass history and `7a File lookup` carries a file's own. Without this the tray's
 * "View journal" item silently does nothing.
 *
 * S8 (#187) rebuilds the tray and can delete this — but only by changing what tray.rs emits, and a
 * tray that emits a dead id is not something the frontend can be trusted to notice on its own.
 */
export const ROUTE_ALIASES = { history: "activity" };

/** Resolve an id that may be a legacy alias. Unknown ids come back unchanged, for the caller to reject. */
export const resolveRoute = (id) => ROUTE_ALIASES[id] ?? id;

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
// (setup succeeded but Proton sign-in lapsed): onboarding can't fix expired auth, and its step-4
// status line already promises a hand-off once the state moves past `firstRun`, so we must actually
// hand off to the main screen's Re-authenticate action rather than trap the user in the wizard.
//
// Entry has two triggers: (1) `firstRun` — the canonical signal (a reachable daemon that has never
// synced), preserving the original single-hook behaviour; and (2) a genuinely fresh machine — a
// *completed* status poll reports the daemon unreachable AND no folder pair is configured yet.
// `statusPolled` gates this so we never claim "fresh" from the pre-poll default state, and
// `configLoaded` gates it so a daemon configured elsewhere (or a config file not yet read) doesn't
// flash the wizard. Pure and side-effect free so it can be unit-tested.
export function nextOnboardingLatch(prev, daemonState, hasConfigPair, configLoaded, statusPolled) {
  // Reachable in any form — hand back to the main screen (which surfaces the right per-state action).
  if (
    daemonState === "idle" ||
    daemonState === "running" ||
    daemonState === "paused" ||
    daemonState === "authExpired"
  )
    return false;
  // Reachable daemon that has never synced: the original firstRun takeover.
  if (daemonState === "firstRun") return true;
  // Fresh machine: a completed poll says the daemon is unreachable AND no folders are chosen.
  if (daemonState === "unreachable" && statusPolled && configLoaded && !hasConfigPair) return true;
  // Anything else (notably `unreachable` right after step 2 wrote the config, or before the first
  // poll): hold whatever we were doing so the flow isn't interrupted.
  return prev;
}
