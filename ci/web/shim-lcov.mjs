// Merges the Istanbul counters the harness saved from every page and worker into one LCOV file.
// node shim-lcov.mjs <counters dir> <lcov file>
import { readdirSync, readFileSync } from "node:fs";
import { basename, dirname, join, resolve } from "node:path";
import libCoverage from "istanbul-lib-coverage";
import libReport from "istanbul-lib-report";
import reports from "istanbul-reports";

const [counters, lcov] = process.argv.slice(2);
const map = libCoverage.createCoverageMap({});
for (const name of readdirSync(counters).filter((name) => name.endsWith(".json"))) {
  map.merge(JSON.parse(readFileSync(join(counters, name), "utf8")));
}
// Paths relative to the repository root, as Sonar and Codecov resolve them.
const projectRoot = resolve(import.meta.dirname, "../..");
const context = libReport.createContext({ dir: dirname(lcov), coverageMap: map });
reports.create("lcovonly", { file: basename(lcov), projectRoot }).execute(context);
