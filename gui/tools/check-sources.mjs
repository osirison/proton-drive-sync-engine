// No invisible characters in any source file.
//
// WHY THIS EXISTS. S3 shipped a literal NUL (U+0000) inside a template string — `${path}\0${dir}`,
// written as the byte rather than as the escape — and every gate stayed green: prettier formatted
// the file, eslint linted it, the tests passed, and the fidelity harness rendered it. What broke was
// everything ABOUT the file. Git classifies a blob containing a NUL as binary, so `git diff` printed
// `Binary files differ` for a 600-line screen, `git blame` and `git grep` returned nothing, and a
// review agent handed the diff could not read the code at all. The tell was a `grep` over the file
// silently finding no matches, which reads as "the code is not there" rather than as a broken file.
//
// So the class is: a character that changes what tooling does and that nobody can see. NUL is the
// severe one; the rest are here because they are the same failure with a smaller blast radius — a
// no-break space or a zero-width joiner inside a copy-deck sentence makes the copy gate compare two
// strings that look identical and are not, and the diff of the fix looks like no change at all.
//
// Deliberately NOT a lint rule: eslint parses JavaScript, and this has to hold for CSS, HTML and
// JSON too — the copy deck's punctuation is only half of it.

import { readdirSync, readFileSync, statSync } from "node:fs";
import { dirname, extname, join, relative, resolve } from "node:path";
import { fileURLToPath } from "node:url";

const HERE = dirname(fileURLToPath(import.meta.url));
const GUI = resolve(HERE, "..");
const ROOTS = ["src", "test", "tools"].map((dir) => join(GUI, dir));
const TEXT = new Set([".js", ".mjs", ".cjs", ".css", ".html", ".json", ".md"]);
/** The prototype's own extraction, which legitimately carries whatever the drawing carried. */
const SKIP = new Set(["frames"]);

/**
 * C0 and C1 controls except tab, newline and carriage return, plus the invisible spaces and joiners
 * that survive a copy-paste. The BOM is included wherever it appears: harmless at offset 0 in some
 * formats, a bug everywhere else in a file.
 *
 * WRITTEN AS ESCAPES, and the first version of this file was not — it spelled the characters out in
 * the pattern and in the table below, so the check failed on itself, nineteen times. Which is the
 * whole argument for the rule: they are invisible, so the only safe way to write one is to write it
 * as something visible.
 */
// The control characters ARE the subject here. `no-control-regex` exists to stop one reaching a
// pattern by accident, which is the bug this scans every other file for; `copy-gate.mjs` sidesteps
// the rule with a printable pilcrow because there it had a choice, and here there is none.
// eslint-disable-next-line no-control-regex
const INVISIBLE = /[\u0000-\u0008\u000B\u000C\u000E-\u001F\u007F-\u009F\u00A0\u200B-\u200D\uFEFF]/g;

const NAMES = {
  "\u0000": "NUL",
  "\u00A0": "NO-BREAK SPACE",
  "\u200B": "ZERO WIDTH SPACE",
  "\u200C": "ZERO WIDTH NON-JOINER",
  "\u200D": "ZERO WIDTH JOINER",
  "\uFEFF": "BYTE ORDER MARK",
};
const name = (ch) => NAMES[ch] ?? `U+${ch.codePointAt(0).toString(16).toUpperCase().padStart(4, "0")}`;

function* walk(dir) {
  for (const entry of readdirSync(dir)) {
    if (SKIP.has(entry) || entry === "node_modules") continue;
    const path = join(dir, entry);
    if (statSync(path).isDirectory()) yield* walk(path);
    else if (TEXT.has(extname(entry))) yield path;
  }
}

const failures = [];
let scanned = 0;
for (const root of ROOTS) {
  for (const path of walk(root)) {
    scanned++;
    const source = readFileSync(path, "utf8");
    for (const match of source.matchAll(INVISIBLE)) {
      const before = source.slice(0, match.index);
      const line = before.split("\n").length;
      const column = match.index - before.lastIndexOf("\n");
      failures.push(
        `${relative(GUI, path)}:${line}:${column} — ${name(match[0])}. Write it as an escape ` +
          `(\\u${match[0].codePointAt(0).toString(16).padStart(4, "0")}) if it is meant, or delete it.`,
      );
    }
  }
}

console.log(`check-sources: ${scanned} files scanned, ${failures.length} invisible character(s)`);
if (failures.length) {
  for (const failure of failures) console.error(`  ${failure}`);
  process.exit(1);
}
