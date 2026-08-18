// Verifies the React+TSX Vite build actually produced static web assets.
import assert from "node:assert/strict";
import { existsSync } from "node:fs";

assert.ok(existsSync("dist/index.html"), "expected dist/index.html to exist after `npm run build`");
console.log("ok: dist/index.html exists");
