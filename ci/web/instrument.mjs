// Writes the shim at argv[2] with Istanbul counters to every later path, keyed by the shim's own path so that the counters of every copy merge into it.
import { readFileSync, writeFileSync } from "node:fs";
import { resolve } from "node:path";
import { createInstrumenter } from "istanbul-lib-instrument";

const [source, ...targets] = process.argv.slice(2);
const path = resolve(source);
const instrumented = createInstrumenter({ esModules: true }).instrumentSync(
  readFileSync(path, "utf8"),
  path,
);
for (const target of targets) writeFileSync(target, instrumented);
