// Dependency-free "lint" stand-in (no eslint/npm install required): scans this
// project's source files for a single banned pattern, `console.log(`, and exits
// non-zero reporting the exact file and line number of each violation, mirroring
// how a real linter reports failures.
//
// Deliberately excludes *.test.js (test files are allowed to log for readability)
// and itself, and only checks source files, so the failure is specific to index.js.

import { readFileSync, readdirSync } from "node:fs";
import { fileURLToPath } from "node:url";
import path from "node:path";

const dir = path.dirname(fileURLToPath(import.meta.url));
const RULE = "no-console-log";
const PATTERN = /console\.log\(/;

const sourceFiles = readdirSync(dir).filter(
  (name) => name.endsWith(".js") && !name.endsWith(".test.js") && name !== "lint.js"
);

let violations = [];

for (const file of sourceFiles) {
  const fullPath = path.join(dir, file);
  const lines = readFileSync(fullPath, "utf8").split("\n");
  lines.forEach((line, index) => {
    if (PATTERN.test(line)) {
      violations.push({ file, line: index + 1, text: line.trim() });
    }
  });
}

if (violations.length > 0) {
  console.error(`lint error [${RULE}]: console.log is not allowed in source files\n`);
  for (const v of violations) {
    console.error(`  ${v.file}:${v.line}  ${v.text}`);
  }
  console.error(`\n${violations.length} problem(s) (${RULE})`);
  process.exit(1);
}

console.log("lint passed: no violations found");
