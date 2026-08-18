// Verifies the Vite build actually produced static web assets (this
// fixture's whole purpose: prove `paws ci` detects Vite and that a real
// `vite build` genuinely runs, not just that the command was invoked).
import assert from "node:assert/strict";
import { existsSync } from "node:fs";

assert.ok(existsSync("dist/index.html"), "expected dist/index.html to exist after `npm run build`");
console.log("ok: dist/index.html exists");
