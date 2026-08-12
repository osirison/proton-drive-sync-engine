// Formatters (F7). Every number and duration the user sees passes through here, so the voice rules
// in `13-copy-deck.md` are enforced once instead of remembered eight times.
//
// The rules these encode, and what each one refuses to do:
//
//   2. "Consequences in things you'd miss" — `1,204 photos, 8.4 GB`, not `directory, recursive`.
//      So bytes and counts are always human, always separated, never raw.
//   4. "Never paraphrase a daemon error" — there is deliberately NO formatter for an error string.
//      A daemon message is shown exactly, in mono. If you find yourself wanting `formatError`,
//      the answer is a <code> element.
//   7. "kept" not "preserved", "waiting" not "pending", "brought here" not "downloaded" IN PROSE.
//      outcomeOf() is the prose tier and obeys it; the mono tier may still say uploaded/downloaded,
//      which is why the engine's own action names are not rewritten at the source.

/** Thousands separators, always. `12,480 files` — rule 2. */
export const count = (n) => (n == null ? EM_DASH : Number(n).toLocaleString("en-GB"));

export const EM_DASH = "—";

const CARDINALS = ["zero", "One", "Two", "Three", "Four", "Five", "Six", "Seven", "Eight", "Nine", "Ten"];

/**
 * A small count spelled out for the START OF A SENTENCE — and nowhere else. Capitalised from `One`
 * up; `zero` is the deliberate exception, and the paragraph below says why.
 *
 * The attention band's two drawn rows are `One file changed on both sides` and `Two deletions are
 * waiting on you`. Both open with a word, both are 13.5px sans prose, and both are the only place
 * in the deck that spells a number. Everything else — the chip's `3 waiting`, the headline's
 * `Syncing 3 changes`, every mono sub-line — uses `count()`, which is why this does not replace it.
 *
 * `zero` is lower-cased on purpose: nothing draws it (a band row exists only when its category has
 * something waiting), and a capital `Zero` at the front of a sentence nobody can reach would read
 * as a supported form. Above ten it hands back to `count()` — `11-notifications.md` writes
 * `5 files changed on both sides` for the grouped banner, so the design itself stops spelling
 * somewhere; eleven is where English style guides put the line and no frame tests it.
 */
/**
 * Pick the singular or the plural wording for a count.
 *
 * Every drawn instance of every counted sentence in the deck happens to be plural — `Syncing 3
 * changes`, `7 changes have piled up`, `3 things need you`, `4 changes are waiting` — so the
 * templates were written with the plural baked in and rendered `1 other changes are waiting on you`
 * the first time a screen fed them a live count. Six sentences, all reachable: one conflict, one
 * queued change, one thing needing you.
 *
 * A whole wording rather than a suffix, because English agreement is not a trailing `s`: `1 change
 * IS waiting` against `3 changes ARE waiting`, `1 thing NEEDS you` against `3 things NEED you`.
 */
export const plural = (n, one, many) => (Number(n) === 1 ? one : many);

export function cardinal(n, register = "sentence") {
  if (n == null) return EM_DASH;
  const value = Number(n);
  if (!Number.isInteger(value) || value < 0 || value >= CARDINALS.length) return count(n);
  const word = CARDINALS[value];
  // S5 is the first mid-sentence use, and the paragraph above is why it needs a register rather
  // than a caller-side `.toLowerCase()`: `7a Never synced`'s band draws BOTH forms in one sentence
  // — "Two match a rule you wrote; two can't be synced at all." Above ten `cardinal` has already
  // handed back to `count()`, whose digits lower-case to themselves, so the register only ever
  // touches the spelled forms.
  return register === "mid" ? word.toLowerCase() : word;
}

/**
 * An em-dash means UNKNOWN and only that. The three plan counts live inside a nullable
 * `last_plan_summary`, so a null summary means "not measured", never zero — rendering `0` there
 * claims a fact the daemon never reported. 14-behaviour-and-state.md is explicit about it.
 */
export const dash = (v) => (v == null ? EM_DASH : String(v));

/**
 * Bytes, in the units the deck uses: `8.4 GB`, `2.8 MB`, `96 KB`, `41.2 GB`.
 *
 * Decimal (1000), not binary (1024), because the deck's own figures are decimal and because the
 * number beside `1,204 photos` is a consequence a person weighs, not a disk-allocation fact. One
 * decimal place above KB, none below — `8.4 GB` reads as a size, `8.43 GB` reads as telemetry.
 */
export function bytes(n) {
  if (n == null) return EM_DASH;
  const size = Number(n);
  if (!Number.isFinite(size)) return EM_DASH;
  if (size < 1000) return `${Math.round(size)} B`;
  const units = ["KB", "MB", "GB", "TB"];
  let value = size / 1000;
  let unit = 0;
  while (value >= 1000 && unit < units.length - 1) {
    value /= 1000;
    unit++;
  }
  // KB stays whole: the deck writes `96 KB`, not `96.0 KB`. So does a whole number of anything —
  // `9a Folders` draws `500 GB` and `9a Review` `214 GB`, and `.0` there is the rule misfiring, not
  // a size someone wrote.
  if (unit === 0) return `${Math.round(value)} ${units[unit]}`;
  const shown = value.toFixed(1);
  return `${shown.endsWith(".0") ? shown.slice(0, -2) : shown} ${units[unit]}`;
}

/**
 * A file's own size, spelled out under 1 KB: `41 bytes`, then `96 KB`, `8.4 GB`.
 *
 * The conflict cards' metadata row is the only place the design writes the word — `41 bytes` and
 * `44 bytes` on the two version cards, and nowhere else in all 51 frames. `bytes()` says `41 B`
 * there, which is right for a transfer row and wrong beside `4 lines · edited 14:38`, where the
 * row is reading out a small text file's facts in words.
 *
 * It hands over to `bytes()` at 1 KB rather than spelling all the way up, because voice rule 2 is
 * "consequences in things you'd miss" and `1,200,000 bytes` is not one of them.
 */
export function fileSize(n) {
  if (n == null) return EM_DASH;
  const size = Number(n);
  if (!Number.isFinite(size)) return EM_DASH;
  return size < 1000 ? `${count(Math.round(size))} bytes` : bytes(size);
}

/**
 * Relative time in the deck's own vocabulary: `2 minutes ago`, `14 seconds ago`, `22m ago`.
 *
 * Two registers, because the design uses both. `long` is prose ("last synced 2 minutes ago") and
 * `short` is the mono tier ("deleted on Proton 22m ago", "14s ago"). Passing the wrong one is the
 * kind of drift a single module exists to prevent.
 */
export function since(epochSecs, register = "long") {
  if (epochSecs == null) return EM_DASH;
  const delta = Math.max(0, Math.floor(Date.now() / 1000) - Number(epochSecs));
  const short = register === "short";
  const units = [
    [86400, "d", "day"],
    [3600, "h", "hour"],
    [60, "m", "minute"],
    [1, "s", "second"],
  ];
  for (const [secs, abbr, word] of units) {
    if (delta >= secs || secs === 1) {
      const n = Math.floor(delta / secs);
      if (short) return `${n}${abbr} ago`;
      return `${n} ${word}${n === 1 ? "" : "s"} ago`;
    }
  }
  return EM_DASH;
}

/**
 * A wall-clock time, `13:20` — the deck's own form in `Paused` / `7 changes have piled up since
 * 13:20.` and in `retrying in 40s · last reached 13:58`.
 *
 * 24-hour and zero-padded, which is what both drawn instances are, and the machine's own timezone,
 * because "since 13:20" is a claim about the user's afternoon. That makes it the one formatter whose
 * output a fixture may not derive — an epoch rendered as a clock time moves across a timezone and
 * across midnight, so `clock.js` requires absolute times to be written literally instead.
 */
export function clock(epochSecs) {
  if (epochSecs == null) return EM_DASH;
  const value = Number(epochSecs);
  if (!Number.isFinite(value)) return EM_DASH;
  return new Date(value * 1000).toLocaleTimeString("en-GB", {
    hour: "2-digit",
    minute: "2-digit",
    hour12: false,
  });
}

/**
 * The notification header's time, and the ONLY register in the app that has a word for "just now".
 *
 * `11a In situ` draws all three forms at once, which is what makes this a formatter rather than a
 * caller's choice: `now` on the banner that has just arrived, `2m ago` on the one behind it, `14:12`
 * on the oldest. Two thresholds, both read off that frame:
 *
 *   · under a minute → `now`. `since(…, "short")` says `0s ago` there, and a banner that counts
 *     seconds at you is the opposite of what this surface is for — it is a thing you glance at.
 *   · under an hour → `since(…, "short")`, the mono relative tier the rest of the app uses.
 *   · beyond that → `clock()`. `14:12` is a time of day, and by then that is the more useful fact.
 *
 * The hour boundary is the one number here no frame pins — `2m ago` and `14:12` are 58 minutes
 * apart at the closest and the drawn banners could be hours apart. It is chosen for the reason the
 * form changes at all: past an hour, "73m ago" is arithmetic and "14:12" is an answer.
 */
export function notifyTime(epochSecs) {
  if (epochSecs == null) return EM_DASH;
  const value = Number(epochSecs);
  if (!Number.isFinite(value)) return EM_DASH;
  const delta = Math.floor(Date.now() / 1000) - value;
  // A clock skew or a fixture reading a hair into the future is `now`, never a negative age.
  if (delta < 60) return "now";
  if (delta < 3600) return since(value, "short");
  return clock(value);
}

/**
 * A month and a year, `Jan 2026` — the register the deletion card's facts strip uses for anything
 * older than a relative time is worth stating in (`last edited Jan 2026`, `last opened Mar 2024`).
 *
 * The SECOND formatter a fixture may not derive, and for `clock`'s reason one unit up: a month
 * boundary is a timezone away for a few hours twice a year, so `4a Deletions` draws a month this
 * cannot promise to reproduce. It is safe for the fidelity gate anyway — the strip is mono, every
 * `Mmm YYYY` is eight characters, and the drawn node is 132px wide whichever month it names — but
 * the string itself is not something the copy gate may assert, exactly as `CONFLICTS.edited` is not.
 *
 * `en-GB` with `month: "short"` is `Jan 2026`, which is the deck's form; `en-US` would give the
 * same, and the locale is pinned for the same reason every other formatter here pins it.
 */
export function monthYear(epochSecs) {
  if (epochSecs == null) return EM_DASH;
  const value = Number(epochSecs);
  if (!Number.isFinite(value)) return EM_DASH;
  return new Date(value * 1000).toLocaleDateString("en-GB", { month: "short", year: "numeric" });
}

/** `about 17 minutes left` · `about 25 minutes to finish` — deliberately vague, never a countdown. */
export function roughly(seconds, tail = "left") {
  if (seconds == null || !Number.isFinite(Number(seconds))) return EM_DASH;
  const mins = Math.max(1, Math.round(Number(seconds) / 60));
  if (mins < 60) return `about ${mins} minute${mins === 1 ? "" : "s"} ${tail}`;
  const hours = Math.round(mins / 60);
  return `about ${hours} hour${hours === 1 ? "" : "s"} ${tail}`;
}

/**
 * The engine's action vocabulary in plain English — the deck's "Outcomes" and "Row outcomes" lines.
 *
 * This is where voice rule 7 is actually enforced: the engine says `download`, the prose says
 * `brought here`. Two registers again, because the Plan screen and the Activity rows word the same
 * action differently ("sent to Proton" vs "brought here" against "brought to this computer").
 *
 * An unknown action returns null rather than a guess. A sync tool inventing a description of what it
 * did to a file is the one place a plausible-sounding fallback is worse than a blank.
 */
// KEYED ON WHAT THE ENGINE ACTUALLY EMITS. `SyncAction` is `#[serde(rename_all = "snake_case")]`
// over the variant names (src/sync.rs), so a planned action's `action` field is
// `create_remote_directory`, not `remote_mkdir`; `move_local`, not `local_move`; `skip_unsupported`,
// not `skip`. Five of the nine keys here were the shorter names nobody emits, and because an unknown
// action deliberately returns null rather than guessing, they failed by drawing a BLANK outcome —
// no error, no warning, just a row that says nothing about what happened to your file.
//
// Nothing caught it because nothing had yet put an engine-shaped plan in front of this function: F7
// wrote the table and the screens that feed it are S4 and S5. F9's `5a Plan` fixture did, and two of
// its nine rows came back empty.
const OUTCOMES = {
  upload: { plan: "sent to Proton", row: "sent to Proton" },
  download: { plan: "brought to this computer", row: "brought here" },
  create_remote_directory: { plan: "folder created on Proton", row: "folder created on Proton" },
  create_local_directory: { plan: "folder created here", row: "folder created here" },
  // The two moves are mirrors, not one outcome: `move_local` applies a rename made on Proton to the
  // local copy (the drawn row); `move_remote` applies a rename made here to Proton, where "to match
  // Proton" would say the opposite of what happens. No frame draws `move_remote`, so it went
  // undetected until S4 rendered it. DEVIATIONS §76.
  // The plan register has to name both ends — `moved on Proton to match` trails off — and says
  // `this computer`, never a brand or OS name (voice rule 6). The row register stays terse.
  move_remote: { plan: "moved on Proton to match this computer", row: "moved on Proton" },
  move_local: { plan: "moved to match Proton", row: "moved to match" },
  conflict: { plan: "both copies kept, nothing lost", row: "both copies kept" },
  remote_delete: { plan: "deleted for good on Proton", row: "deleted for good on Proton" },
  skip_unsupported: { plan: "skipped, can't be synced", row: "skipped, can't be synced" },

  // The four F7 left null, chosen by S4 rather than measured: no frame draws an outcome for any of
  // them and `13-copy-deck.md` carries none. They are here because a screen now draws every row of a
  // plan, and the fallback is a labelless row — `local_delete` would be the destructive row, tinted,
  // with no sentence. Each is the narrowest true statement rather than a translation of the engine's
  // noun, and each is recorded in DEVIATIONS §76 as chosen copy the deck can overrule.
  local_delete: { plan: "deleted for good on this computer", row: "deleted for good here" },
  purge: { plan: "record cleared, no file touched", row: "record cleared" },
  auto_link: { plan: "already matching, linked up", row: "linked up" },
  type_conflict: { plan: "a folder here, a file there — nothing moves", row: "a folder here, a file there" },
};

export function outcomeOf(action, register = "row") {
  const entry = OUTCOMES[action];
  return entry ? entry[register] : null;
}
