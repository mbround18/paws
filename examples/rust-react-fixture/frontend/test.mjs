// Verifies the Vite build produced static assets the Axum backend expects
// to find under frontend/dist (see ../src/main.rs's ServeDir).
import assert from "node:assert/strict";
import { existsSync } from "node:fs";

assert.ok(existsSync("dist/index.html"), "expected dist/index.html to exist after `npm run build`");
console.log("ok: dist/index.html exists");
