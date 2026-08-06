// Are the committed fixtures still what the prototype produces?
//
// NOT a `git diff`. Re-extracting on another machine moves text boxes by hundredths of a pixel —
// sub-pixel layout is not identical across platforms even with the same font files — so a byte
// comparison reports the platform, not staleness. CI failed on exactly that, one commit after the
// same mistake cost 187 phantom colour failures.
//
// So this compares the way the gate compares: the same tolerances, from the same module. A prototype
// edit that moves a real number fails; a different machine does not.

import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";
import { STYLE_PROPS, SVG_ATTRS, compare, valueOf, LENGTH_TOLERANCE_PX, boxComparability } from "./props.mjs";

const HERE = dirname(fileURLToPath(import.meta.url));
const COMMITTED = join(HERE, "frames");
const fresh = mkdtempSync(join(tmpdir(), "fidelity-"));

const run = spawnSync(process.execPath, [join(HERE, "extract.mjs"), fresh], { stdio: "inherit" });
if (run.status !== 0) {
  rmSync(fresh, { recursive: true, force: true });
  process.exit(run.status ?? 1);
}

const load = (dir, file) => JSON.parse(readFileSync(join(dir, file), "utf8"));
const drift = [];

const committedFiles = readdirSync(COMMITTED).filter((f) => f.endsWith(".json") && f !== "index.json");
const freshFiles = readdirSync(fresh).filter((f) => f.endsWith(".json") && f !== "index.json");

for (const file of new Set([...committedFiles, ...freshFiles])) {
  if (!committedFiles.includes(file)) {
    drift.push(`${file}: the prototype draws a frame the fixtures do not have`);
    continue;
  }
  if (!freshFiles.includes(file)) {
    drift.push(`${file}: the fixtures have a frame the prototype no longer draws`);
    continue;
  }
  const was = load(COMMITTED, file);
  const now = load(fresh, file);
  const byKey = new Map(was.nodes.map((n) => [n.key, n]));
  // Computed from the COMMITTED side: the question is whether this fixture's box was ever a
  // machine-independent number, not whether today's render happens to contain a glyph.
  const boxComparable = boxComparability(was.nodes);

  if (was.nodes.length !== now.nodes.length) {
    drift.push(`${was.label}: ${was.nodes.length} nodes committed, ${now.nodes.length} drawn`);
    continue;
  }
  for (const node of now.nodes) {
    const old = byKey.get(node.key);
    if (!old) {
      drift.push(`${was.label} · ${node.key}: a node key the fixtures do not have`);
      continue;
    }
    for (const prop of STYLE_PROPS) {
      const reason = compare(prop, valueOf(old.styles, prop), valueOf(node.styles, prop));
      if (reason) drift.push(`${was.label} · ${node.key} · ${prop}: ${reason}`);
    }
    for (const attr of SVG_ATTRS) {
      if ((old.svgAttrs?.[attr] ?? null) !== (node.svgAttrs?.[attr] ?? null)) {
        drift.push(
          `${was.label} · ${node.key} · @${attr}: ${old.svgAttrs?.[attr]} vs ${node.svgAttrs?.[attr]}`,
        );
      }
    }
    for (const side of boxComparable(node) ? ["w", "h"] : []) {
      if (Math.abs(old.box[side] - node.box[side]) > LENGTH_TOLERANCE_PX) {
        drift.push(`${was.label} · ${node.key} · box.${side}: ${old.box[side]} vs ${node.box[side]}`);
      }
    }
    // Copy is exact — a changed sentence is never sub-pixel noise, and the copy gate depends on it.
    for (const field of ["text", "fullText"]) {
      if ((old[field] ?? null) !== (node[field] ?? null)) {
        drift.push(`${was.label} · ${node.key} · ${field}: "${old[field]}" vs "${node[field]}"`);
      }
    }
  }
}

rmSync(fresh, { recursive: true, force: true });

if (drift.length) {
  console.error(`\nfidelity:stale — the prototype has moved and the fixtures have not:\n`);
  for (const d of drift.slice(0, 30)) console.error(`  ${d}`);
  if (drift.length > 30) console.error(`  … and ${drift.length - 30} more`);
  console.error(`\nRun \`npm run fidelity:extract\` and commit the result.`);
  process.exit(1);
}
console.log(`fidelity:stale — ${committedFiles.length} fixtures still match the prototype`);
