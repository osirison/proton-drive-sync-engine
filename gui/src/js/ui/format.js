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
  // KB stays whole: the deck writes `96 KB`, not `96.0 KB`.
  return unit === 0 ? `${Math.round(value)} ${units[unit]}` : `${value.toFixed(1)} ${units[unit]}`;
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
  move_remote: { plan: "moved to match Proton", row: "moved to match" },
  move_local: { plan: "moved to match Proton", row: "moved to match" },
  conflict: { plan: "both copies kept, nothing lost", row: "both copies kept" },
  remote_delete: { plan: "deleted for good on Proton", row: "deleted for good on Proton" },
  skip_unsupported: { plan: "skipped, can't be synced", row: "skipped, can't be synced" },
};

// FOUR VARIANTS THE ENGINE EMITS AND THIS TABLE HAS NO WORDING FOR: `local_delete`, `purge`,
// `auto_link`, `type_conflict`. No frame draws an outcome label for any of them, and `13-copy-deck.md`
// carries none — so the words do not exist yet, and inventing them here would be this module doing
// design. `outcomeOf` returns null, which is the documented safe answer and renders as no label.
//
// Three of the four matter to a screen that is not built yet, and each needs a decision rather than
// a translation: `local_delete` is the mirror of `remote_delete` (S3/S5), `type_conflict` is the
// "a folder here, a file there" case S2 already draws its own copy for, and `purge` is index-only
// cleanup that touches no user data — arguably it should never reach a row at all.

export function outcomeOf(action, register = "row") {
  const entry = OUTCOMES[action];
  return entry ? entry[register] : null;
}
