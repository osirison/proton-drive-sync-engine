// Extract twice, require identical bytes.
//
// Every cross-machine failure this harness has had was an unpinned input, and a single machine is
// the cheapest place to find one: if two runs seconds apart disagree, something is being measured
// that is not the design, and it will disagree far more loudly on another machine.
//
// It has already earned its place. With the animation freeze seeking before pausing, `opacity` under
// `breathe` read 0.45 on one run and 0.450015 on the next — invisible in the gate's ±0.5px world,
// and enough to make the staleness check fail forever on any machine but the one that extracted.

import { mkdtempSync, readFileSync, readdirSync, rmSync } from "node:fs";
import { tmpdir } from "node:os";
import { dirname, join } from "node:path";
import { fileURLToPath } from "node:url";
import { spawnSync } from "node:child_process";

const HERE = dirname(fileURLToPath(import.meta.url));
const runs = [mkdtempSync(join(tmpdir(), "fidelity-a-")), mkdtempSync(join(tmpdir(), "fidelity-b-"))];

for (const dir of runs) {
  const run = spawnSync(process.execPath, [join(HERE, "extract.mjs"), dir], { stdio: "ignore" });
  if (run.status !== 0) {
    runs.forEach((d) => rmSync(d, { recursive: true, force: true }));
    console.error("fidelity:determinism — extract.mjs failed");
    process.exit(1);
  }
}

const [a, b] = runs;
const files = readdirSync(a);
const differing = files.filter((f) => readFileSync(join(a, f), "utf8") !== readFileSync(join(b, f), "utf8"));
runs.forEach((d) => rmSync(d, { recursive: true, force: true }));

if (differing.length) {
  console.error(
    `\nfidelity:determinism — ${differing.length} fixture(s) differ between two runs on ONE machine:\n`,
  );
  for (const f of differing.slice(0, 10)) console.error(`  ${f}`);
  console.error("\nSomething unpinned is being measured. Check the pinning table in the README.");
  process.exit(1);
}
console.log(`fidelity:determinism — two extractions produced ${files.length} identical files`);
